use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DispatchResponse {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub dispatch_version: Option<String>,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub source_dispatch_generated_at_utc: Option<String>,
    #[serde(default)]
    pub summary: Option<DispatchSummary>,
    #[serde(default)]
    pub actions: Vec<DispatchAction>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DispatchSummary {
    #[serde(default)]
    pub total_actions: Option<u64>,
    #[serde(default)]
    pub assigned_actions: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl DispatchSummary {
    pub fn assigned_count(&self, fallback_len: usize) -> usize {
        self.assigned_actions
            .or(self.total_actions)
            .map(|v| v as usize)
            .unwrap_or(fallback_len)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DispatchAction {
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub action_kind: Option<String>,
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
    pub dry_run: Option<bool>,
    #[serde(default)]
    pub ai_context: Option<Value>,
    #[serde(default)]
    pub migration_policy: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[cfg(test)]
mod tests {
    use super::DispatchResponse;

    #[test]
    fn parses_dispatch_response_with_actions() {
        let raw = r#"
        {
          "project_id": "smart-city-campinas",
          "node_id": "fog-01",
          "dispatch_version": "0.2.0",
          "generated_at_utc": "2026-05-19T12:55:53Z",
          "summary": {
            "total_actions": 2,
            "assigned_actions": 1
          },
          "actions": [
            {
              "action_id": "gap:1",
              "action_kind": "ensure_container_running",
              "runtime": "container",
              "artifact": "chirpstack/chirpstack:latest",
              "target_node": "fog-01",
              "assigned_node": "fog-01",
              "dry_run": true
            }
          ]
        }
        "#;

        let parsed: DispatchResponse =
            serde_json::from_str(raw).expect("dispatch response should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.node_id.as_deref(), Some("fog-01"));
        assert_eq!(
            parsed.summary.as_ref().and_then(|s| s.total_actions),
            Some(2)
        );
        assert_eq!(
            parsed.summary.as_ref().and_then(|s| s.assigned_actions),
            Some(1)
        );
        assert_eq!(parsed.actions.len(), 1);
        assert_eq!(parsed.actions[0].action_id.as_deref(), Some("gap:1"));
        assert_eq!(
            parsed.actions[0].artifact.as_deref(),
            Some("chirpstack/chirpstack:latest")
        );
    }
}
