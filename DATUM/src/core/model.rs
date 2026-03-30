use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: Meta,
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub events: Vec<Event>,
    pub actions: Vec<ActionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub schema_version: String,
    pub agent_version: String,
    pub cycle_id: String,
    pub observed_at_utc: DateTime<Utc>,
    pub node_id: String,
    pub host_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub attrs: JsonValue,
    pub observed_at_utc: DateTime<Utc>,
    pub ttl_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub rel: String,
    pub attrs: JsonValue,
    pub observed_at_utc: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub severity: String,
    pub target: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_plugin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
    pub observed_at_utc: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: String,
    pub correlation_id: String,
    pub origin: String,
    pub kind: String,
    pub phase: String,
    pub status: String,
    pub service_id: String,
    pub service_name: String,
    pub target_kind: String,
    pub target_id: String,
    pub reason: Option<String>,
    pub message: Option<String>,
    pub dry_run: bool,
    pub suppressed: bool,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    pub observed_at_utc: DateTime<Utc>,
}
