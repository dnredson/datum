use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentPlanningResponse {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub deployment_planning: DeploymentPlanning,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentPlanning {
    #[serde(default)]
    pub planner_version: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub items: Vec<DeploymentPlanningItem>,
    #[serde(default)]
    pub summary: Option<DeploymentPlanningSummary>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentPlanningItem {
    #[serde(default)]
    pub planning_id: Option<String>,
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub module_kind: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub target_node: Option<String>,
    #[serde(default)]
    pub fallback_to: Option<String>,
    #[serde(default)]
    pub assigned_node: Option<String>,
    #[serde(default)]
    pub assignment_reason: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub required_step: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub ai_context: Option<Value>,
    #[serde(default)]
    pub capability: Option<Value>,
    #[serde(default)]
    pub feedback: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeploymentPlanningSummary {
    #[serde(default)]
    pub total_items: Option<u64>,
    #[serde(default)]
    pub assigned_items: Option<u64>,
    #[serde(default)]
    pub high: Option<u64>,
    #[serde(default)]
    pub medium: Option<u64>,
    #[serde(default)]
    pub low: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::DeploymentPlanningResponse;

    #[test]
    fn parse_response_with_items() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "deployment_planning":{
            "planner_version":"0.1.0",
            "items":[
              {
                "planning_id":"plan-1",
                "required_step":"add_local_mapping",
                "priority":"high",
                "module_id":"chirpstack-primary",
                "artifact":"chirpstack/chirpstack:latest",
                "reason":"client lacks trusted local mapping for this action",
                "unknown_future_field":"ok"
              }
            ],
            "summary":{"total_items":1,"high":1,"medium":0,"low":0}
          }
        }"#;

        let parsed: DeploymentPlanningResponse =
            serde_json::from_str(raw).expect("response should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.node_id.as_deref(), Some("fog-01"));
        assert_eq!(parsed.deployment_planning.items.len(), 1);
        assert_eq!(
            parsed.deployment_planning.items[0].required_step.as_deref(),
            Some("add_local_mapping")
        );
        assert_eq!(
            parsed.deployment_planning.items[0]
                .extra
                .get("unknown_future_field")
                .and_then(|v| v.as_str()),
            Some("ok")
        );
    }

    #[test]
    fn parse_empty_response() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "deployment_planning":{
            "items":[],
            "summary":{"total_items":0,"assigned_items":0,"high":0,"medium":0,"low":0}
          }
        }"#;

        let parsed: DeploymentPlanningResponse =
            serde_json::from_str(raw).expect("empty response should parse");

        assert!(parsed.deployment_planning.items.is_empty());
        let summary = parsed
            .deployment_planning
            .summary
            .expect("summary should exist");
        assert_eq!(summary.total_items, Some(0));
        assert_eq!(summary.assigned_items, Some(0));
    }
}
