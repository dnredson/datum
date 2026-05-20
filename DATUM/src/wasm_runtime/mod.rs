pub mod runner;

use serde::{Deserialize, Serialize};

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
}
