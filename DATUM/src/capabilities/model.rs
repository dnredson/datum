use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeCapabilities {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub updated_at_utc: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub runtimes: RuntimeCapabilities,
    #[serde(default)]
    pub flow: Option<FlowCapabilities>,
    #[serde(default)]
    pub resources: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeCapabilities {
    #[serde(default)]
    pub container: Option<ContainerCapabilities>,
    #[serde(default)]
    pub wasm: Option<WasmCapabilities>,
    #[serde(default)]
    pub process: Option<ProcessCapabilities>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContainerCapabilities {
    pub available: bool,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub allow_execution: Option<bool>,
    #[serde(default)]
    pub supports_existing_start: Option<bool>,
    #[serde(default)]
    pub supports_restart: Option<bool>,
    #[serde(default)]
    pub supports_create: Option<bool>,
    #[serde(default)]
    pub supports_pull: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmCapabilities {
    pub available: bool,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub allow_execution: Option<bool>,
    #[serde(default)]
    pub supports_wasi: Option<bool>,
    #[serde(default)]
    pub supports_http: Option<bool>,
    #[serde(default)]
    pub supports_mqtt: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProcessCapabilities {
    pub available: bool,
    #[serde(default)]
    pub allow_execution: Option<bool>,
    #[serde(default)]
    pub supports_restart_command: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FlowCapabilities {
    #[serde(default)]
    pub supports_mqtt_reconfigure: Option<bool>,
    #[serde(default)]
    pub supports_http_reconfigure: Option<bool>,
    #[serde(default)]
    pub supports_bridge_reconfigure: Option<bool>,
    #[serde(default)]
    pub supports_proxy_reconfigure: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
