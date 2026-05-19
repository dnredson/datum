use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationPlanResponse {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub installation_plan: InstallationPlan,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationPlan {
    #[serde(default)]
    pub installer_version: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub items: Vec<InstallationPlanItem>,
    #[serde(default)]
    pub summary: Option<InstallationPlanSummary>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationPlanItem {
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub planning_id: Option<String>,
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
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
    pub source_planning_item: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallationPlanSummary {
    #[serde(default)]
    pub total_items: Option<u64>,
    #[serde(default)]
    pub operator_required: Option<u64>,
    #[serde(default)]
    pub auto_executable: Option<u64>,
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
    use super::InstallationPlanResponse;

    #[test]
    fn parse_response_with_items() {
        let raw = r#"{
          "project_id":"smart-city-campinas",
          "node_id":"fog-01",
          "installation_plan":{
            "installer_version":"0.1.0",
            "source":"deployment_planning",
            "items":[
              {
                "installation_id":"installation-1",
                "installation_action":"prepare_local_mapping",
                "priority":"high",
                "module_id":"chirpstack-primary",
                "artifact":"chirpstack/chirpstack:latest",
                "requires_operator":true,
                "safe_to_auto_execute":false,
                "unknown_future_field":"ok"
              }
            ],
            "summary":{"total_items":1,"operator_required":1,"auto_executable":0,"high":1,"medium":0,"low":0}
          }
        }"#;

        let parsed: InstallationPlanResponse =
            serde_json::from_str(raw).expect("response should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.node_id.as_deref(), Some("fog-01"));
        assert_eq!(parsed.installation_plan.items.len(), 1);
        assert_eq!(
            parsed.installation_plan.items[0]
                .installation_action
                .as_deref(),
            Some("prepare_local_mapping")
        );
        assert_eq!(
            parsed.installation_plan.items[0]
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
          "installation_plan":{
            "items":[],
            "summary":{"total_items":0,"operator_required":0,"auto_executable":0,"high":0,"medium":0,"low":0}
          }
        }"#;

        let parsed: InstallationPlanResponse =
            serde_json::from_str(raw).expect("empty response should parse");

        assert!(parsed.installation_plan.items.is_empty());
        let summary = parsed
            .installation_plan
            .summary
            .expect("summary should exist");
        assert_eq!(summary.total_items, Some(0));
        assert_eq!(summary.operator_required, Some(0));
        assert_eq!(summary.auto_executable, Some(0));
    }
}
