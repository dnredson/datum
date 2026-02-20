use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: Meta,
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    pub events: Vec<Event>,
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
    pub kind: String, // "host" | "container" | "process" | "check" ...
    pub attrs: JsonValue,
    pub observed_at_utc: DateTime<Utc>,
    pub ttl_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub rel: String, // "runs" | "hosts" | "checks" | ...
    pub attrs: JsonValue,
    pub observed_at_utc: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub kind: String,     // "service_down" | "check_failed" ...
    pub severity: String, // "info" | "warn" | "crit"
    pub target: String,
    pub message: String,
    pub observed_at_utc: DateTime<Utc>,
}
