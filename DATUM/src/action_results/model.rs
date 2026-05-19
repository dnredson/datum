use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ActionExecutionReport {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub dispatch_version: Option<String>,
    #[serde(default)]
    pub action_plan_id: Option<String>,
    #[serde(default)]
    pub dispatch_generated_at_utc: Option<String>,
    #[serde(default)]
    pub reported_at_utc: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub results: Vec<ActionExecutionResult>,
    #[serde(default)]
    pub summary: Option<ActionExecutionSummary>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ActionExecutionResult {
    pub action_id: String,
    #[serde(default)]
    pub action_kind: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub target_node: Option<String>,
    #[serde(default)]
    pub assigned_node: Option<String>,
    pub execution_status: String,
    #[serde(default)]
    pub validation_status: Option<String>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub started_at_utc: Option<String>,
    #[serde(default)]
    pub finished_at_utc: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub capability: Option<Value>,
    #[serde(default)]
    pub ai_context: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ActionExecutionSummary {
    pub total_results: u64,
    #[serde(default)]
    pub dry_run: Option<u64>,
    #[serde(default)]
    pub blocked: Option<u64>,
    #[serde(default)]
    pub skipped: Option<u64>,
    #[serde(default)]
    pub rejected: Option<u64>,
    #[serde(default)]
    pub would_execute: Option<u64>,
    #[serde(default)]
    pub already_running: Option<u64>,
    #[serde(default)]
    pub started: Option<u64>,
    #[serde(default)]
    pub not_installed: Option<u64>,
    #[serde(default)]
    pub missing_local_mapping: Option<u64>,
    #[serde(default)]
    pub docker_unavailable: Option<u64>,
    #[serde(default)]
    pub failed: Option<u64>,
    #[serde(default)]
    pub side_effects_performed: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
