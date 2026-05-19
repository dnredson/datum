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
    pub metadata: Option<Value>,
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

#[cfg(test)]
mod tests {
    use super::{InstallationTask, InstallationTaskList};

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
}
