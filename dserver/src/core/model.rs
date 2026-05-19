use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEnvelope {
    pub protocol_version: String,
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub meta: Meta,
    #[serde(default)]
    pub entities: Value,
    #[serde(default)]
    pub edges: Value,
    #[serde(default)]
    pub events: Value,
    #[serde(default)]
    pub actions: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub node_id: String,
    pub cycle_id: String,
    pub observed_at_utc: String,
    pub schema_version: String,
    #[serde(default)]
    pub host_id: Option<String>,
}
