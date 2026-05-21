use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalDGraphSnapshot {
    pub node_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub runtimes: Vec<LocalRuntimeSnapshot>,
    #[serde(default)]
    pub modules: Vec<LocalModuleSnapshot>,
    #[serde(default)]
    pub flows: Vec<LocalFlowSnapshot>,
    #[serde(default)]
    pub health: Vec<LocalHealthSnapshot>,
    #[serde(default)]
    pub capabilities: Vec<LocalCapabilitySnapshot>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalRuntimeSnapshot {
    pub runtime_id: String,
    pub runtime_kind: String,
    pub status: String,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalModuleSnapshot {
    pub module_id: String,
    #[serde(default)]
    pub module_kind: Option<String>,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub observed_state: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub artifact_ref: Option<String>,
    #[serde(default)]
    pub local_mapping_key: Option<String>,
    #[serde(default)]
    pub local_container_name: Option<String>,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalFlowSnapshot {
    pub flow_id: String,
    #[serde(default)]
    pub flow_kind: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub edges: Vec<LocalFlowEdgeSnapshot>,
    #[serde(default)]
    pub status: Option<String>,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalFlowEdgeSnapshot {
    pub edge_id: String,
    pub from_module_id: String,
    pub to_module_id: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalHealthSnapshot {
    pub target_kind: String,
    pub target_id: String,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalCapabilitySnapshot {
    pub capability_id: String,
    pub capability_kind: String,
    pub supported: bool,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}
