use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationTask {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
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
    pub required_step: Option<String>,
    #[serde(default)]
    pub installation_action: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub requires_operator: Option<bool>,
    #[serde(default)]
    pub safe_to_auto_execute: Option<bool>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub created_at_utc: Option<String>,
    #[serde(default)]
    pub updated_at_utc: Option<String>,
    #[serde(default)]
    pub source_installation_item: Option<Value>,
    #[serde(default)]
    pub installation_policy: Option<InstallationPolicy>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationPolicy {
    #[serde(default)]
    pub policy_version: Option<String>,
    #[serde(default)]
    pub decision: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub requires_operator: Option<bool>,
    #[serde(default)]
    pub safe_to_auto_execute: Option<bool>,
    #[serde(default)]
    pub allowed_future_modes: Option<Vec<String>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationTaskList {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub tasks: Vec<InstallationTask>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationTaskApproval {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub approved: Option<bool>,
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub approved_at_utc: Option<String>,
    #[serde(default)]
    pub approval_mode: Option<String>,
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
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationTaskApprovalList {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub approvals: Vec<InstallationTaskApproval>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationTaskReadiness {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub readiness: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
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
    #[serde(default)]
    pub safe_to_auto_execute: Option<bool>,
    #[serde(default)]
    pub evaluated_at_utc: Option<String>,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    #[serde(default)]
    pub approval_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationTaskReadinessList {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub readiness: Vec<InstallationTaskReadiness>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApprovalRequest {
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub expires_at_utc: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalRequest, InstallationTask, InstallationTaskApproval, InstallationTaskApprovalList,
        InstallationTaskList, InstallationTaskReadiness, InstallationTaskReadinessList,
    };

    #[test]
    fn parse_task_list_with_tasks() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "count":1,
          "tasks":[
            {
              "project_id":"smart-city-campinas",
              "node_id":"fog-01",
              "task_id":"project:smart-city-campinas:node:fog-01:installation:installation:planning:1:prepare_local_mapping",
              "installation_id":"installation:planning:1:prepare_local_mapping",
              "installation_action":"prepare_local_mapping",
              "status":"planned",
              "priority":"high",
              "module_id":"chirpstack-primary",
              "artifact":"chirpstack/chirpstack:latest",
              "requires_operator":true,
              "safe_to_auto_execute":false,
              "future_field":"ok"
            }
          ]
        }"#;

        let parsed: InstallationTaskList =
            serde_json::from_str(raw).expect("task list should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.node_id.as_deref(), Some("fog-01"));
        assert_eq!(parsed.count, Some(1));
        assert_eq!(parsed.tasks.len(), 1);
        assert_eq!(
            parsed.tasks[0].installation_action.as_deref(),
            Some("prepare_local_mapping")
        );
        assert_eq!(
            parsed.tasks[0]
                .extra
                .get("future_field")
                .and_then(|value| value.as_str()),
            Some("ok")
        );
    }

    #[test]
    fn parse_empty_task_list() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "count":0,
          "tasks":[]
        }"#;

        let parsed: InstallationTaskList =
            serde_json::from_str(raw).expect("empty task list should parse");

        assert_eq!(parsed.count, Some(0));
        assert!(parsed.tasks.is_empty());
    }

    #[test]
    fn parse_single_task() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "task_id":"project:smart-city-campinas:node:fog-01:installation:installation:planning:1:prepare_local_mapping",
          "installation_id":"installation:planning:1:prepare_local_mapping",
          "status":"acknowledged",
          "updated_at_utc":"2026-05-19T13:00:00Z"
        }"#;

        let parsed: InstallationTask = serde_json::from_str(raw).expect("task should parse");

        assert_eq!(parsed.status.as_deref(), Some("acknowledged"));
        assert_eq!(
            parsed.updated_at_utc.as_deref(),
            Some("2026-05-19T13:00:00Z")
        );
    }

    #[test]
    fn parse_task_with_installation_policy() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "task_id":"task-1",
          "status":"planned",
          "installation_policy":{
            "policy_version":"0.1.0",
            "decision":"candidate_for_assisted_execution",
            "reason":"artifact installation may be assisted in a future milestone, but is not executable yet",
            "requires_operator":true,
            "safe_to_auto_execute":false,
            "allowed_future_modes":["operator_approved"],
            "future_field":"ok"
          }
        }"#;

        let parsed: InstallationTask = serde_json::from_str(raw).expect("task should parse");

        let policy = parsed
            .installation_policy
            .expect("installation_policy should parse");
        assert_eq!(
            policy.decision.as_deref(),
            Some("candidate_for_assisted_execution")
        );
        assert_eq!(
            policy.allowed_future_modes,
            Some(vec!["operator_approved".to_string()])
        );
        assert_eq!(
            policy
                .extra
                .get("future_field")
                .and_then(|value| value.as_str()),
            Some("ok")
        );
    }

    #[test]
    fn parse_task_without_installation_policy() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "task_id":"task-1",
          "status":"planned"
        }"#;

        let parsed: InstallationTask = serde_json::from_str(raw).expect("task should parse");

        assert!(parsed.installation_policy.is_none());
    }

    #[test]
    fn parse_installation_task_approval() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "task_id":"task-1",
          "approval_id":"approval-1",
          "approved":true,
          "approved_by":"operator",
          "approved_at_utc":"2026-05-19T13:00:00Z",
          "approval_mode":"operator_approved",
          "reason":"approved for future assisted execution",
          "future_field":"ok"
        }"#;

        let parsed: InstallationTaskApproval =
            serde_json::from_str(raw).expect("approval should parse");

        assert_eq!(parsed.task_id.as_deref(), Some("task-1"));
        assert_eq!(parsed.approved, Some(true));
        assert_eq!(parsed.approval_mode.as_deref(), Some("operator_approved"));
        assert_eq!(
            parsed
                .extra
                .get("future_field")
                .and_then(|value| value.as_str()),
            Some("ok")
        );
    }

    #[test]
    fn parse_installation_task_approval_list() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "count":1,
          "approvals":[
            {
              "task_id":"task-1",
              "approval_id":"approval-1",
              "approved":true
            }
          ]
        }"#;

        let parsed: InstallationTaskApprovalList =
            serde_json::from_str(raw).expect("approval list should parse");

        assert_eq!(parsed.count, Some(1));
        assert_eq!(parsed.approvals.len(), 1);
        assert_eq!(
            parsed.approvals[0].approval_id.as_deref(),
            Some("approval-1")
        );
    }

    #[test]
    fn approval_request_serializes_optional_fields() {
        let request = ApprovalRequest {
            approved_by: Some("operator".to_string()),
            reason: Some("approved for future assisted execution".to_string()),
            expires_at_utc: Some("2026-06-01T00:00:00Z".to_string()),
        };

        let value = serde_json::to_value(request).expect("request should serialize");

        assert_eq!(value["approved_by"].as_str(), Some("operator"));
        assert_eq!(
            value["reason"].as_str(),
            Some("approved for future assisted execution")
        );
        assert_eq!(
            value["expires_at_utc"].as_str(),
            Some("2026-06-01T00:00:00Z")
        );
    }

    #[test]
    fn parse_installation_task_readiness() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "task_id":"task-1",
          "readiness":"ready_for_assisted_execution",
          "reason":"task has candidate policy and operator approval metadata",
          "task_status":"acknowledged",
          "policy_decision":"candidate_for_assisted_execution",
          "approval_status":"approved",
          "approval_id":"approval-1",
          "approved_at_utc":"2026-05-19T13:00:00Z",
          "safe_to_auto_execute":false,
          "future_field":"ok"
        }"#;

        let parsed: InstallationTaskReadiness =
            serde_json::from_str(raw).expect("readiness should parse");

        assert_eq!(
            parsed.readiness.as_deref(),
            Some("ready_for_assisted_execution")
        );
        assert_eq!(parsed.approval_status.as_deref(), Some("approved"));
        assert_eq!(parsed.safe_to_auto_execute, Some(false));
        assert_eq!(
            parsed
                .extra
                .get("future_field")
                .and_then(|value| value.as_str()),
            Some("ok")
        );
    }

    #[test]
    fn parse_installation_task_readiness_list() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "count":1,
          "readiness":[
            {
              "task_id":"task-1",
              "readiness":"not_ready",
              "approval_status":"missing"
            }
          ]
        }"#;

        let parsed: InstallationTaskReadinessList =
            serde_json::from_str(raw).expect("readiness list should parse");

        assert_eq!(parsed.count, Some(1));
        assert_eq!(parsed.readiness.len(), 1);
        assert_eq!(parsed.readiness[0].task_id.as_deref(), Some("task-1"));
        assert_eq!(
            parsed.readiness[0].approval_status.as_deref(),
            Some("missing")
        );
    }
}
