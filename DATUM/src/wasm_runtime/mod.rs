pub mod artifact;
pub mod client;
pub mod integrity;
pub mod registry;
pub mod report;
pub mod runner;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmRuntimeLimits {
    pub fuel: Option<u64>,
    pub timeout_ms: Option<u64>,
}

impl Default for WasmRuntimeLimits {
    fn default() -> Self {
        Self {
            fuel: Some(1_000_000),
            timeout_ms: Some(1_000),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WasmRuntimeLimitOutcome {
    pub fuel_configured: Option<u64>,
    pub fuel_consumed: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub limit_exceeded: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmRuntimeInput {
    pub module_name: String,
    pub function_name: String,
    pub args: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmRuntimeResult {
    pub module_name: String,
    pub function_name: String,
    pub status: String,
    pub result_i32: Option<i32>,
    pub message: Option<String>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
    pub filesystem_access: bool,
    pub network_access: bool,
    pub host_command_access: bool,
    pub docker_access: bool,
    pub limits: WasmRuntimeLimits,
    pub limit_outcome: WasmRuntimeLimitOutcome,
}
