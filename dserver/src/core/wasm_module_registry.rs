use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MODULE_KIND_EMBEDDED_TEST: &str = "embedded_test";
const MODULE_KIND_DIAGNOSTIC: &str = "diagnostic";
const MODULE_KIND_REMEDIATION_CANDIDATE: &str = "remediation_candidate";
const MODULE_KIND_UNKNOWN: &str = "unknown";

const STATUS_REGISTERED: &str = "registered";
const STATUS_DISABLED: &str = "disabled";
const STATUS_DEPRECATED: &str = "deprecated";
const STATUS_BLOCKED: &str = "blocked";

const EXECUTION_MODE_EMBEDDED_TEST: &str = "embedded_test";
const EXECUTION_MODE_NO_OP: &str = "no_op";
const EXECUTION_MODE_DRY_RUN: &str = "dry_run";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmModuleDescriptor {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub module_id: String,
    #[serde(default)]
    pub module_name: String,
    #[serde(default)]
    pub module_version: Option<String>,
    #[serde(default)]
    pub module_kind: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub allowed_functions: Vec<WasmFunctionDescriptor>,
    #[serde(default)]
    pub artifact_ref: Option<WasmArtifactRef>,
    #[serde(default)]
    pub safety: WasmModuleSafetyMetadata,
    #[serde(default)]
    pub created_at_utc: Option<String>,
    #[serde(default)]
    pub updated_at_utc: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmFunctionDescriptor {
    #[serde(default)]
    pub function_name: String,
    #[serde(default)]
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default)]
    pub allowed_execution_modes: Vec<String>,
    #[serde(default)]
    pub max_fuel: Option<u64>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmArtifactRef {
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub artifact_uri: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub signature_ref: Option<String>,
    #[serde(default)]
    pub provenance: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmModuleSafetyMetadata {
    #[serde(default)]
    pub filesystem_access: bool,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub host_command_access: bool,
    #[serde(default)]
    pub docker_access: bool,
    #[serde(default)]
    pub artifact_access: bool,
    #[serde(default)]
    pub installation_task_access: bool,
    #[serde(default)]
    pub assisted_execution_access: bool,
    #[serde(default)]
    pub side_effects_allowed: bool,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmModuleDescriptorList {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub modules: Vec<WasmModuleDescriptor>,
}

fn is_valid_module_kind(kind: &str) -> bool {
    matches!(
        kind,
        MODULE_KIND_EMBEDDED_TEST
            | MODULE_KIND_DIAGNOSTIC
            | MODULE_KIND_REMEDIATION_CANDIDATE
            | MODULE_KIND_UNKNOWN
    )
}

fn is_valid_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_REGISTERED | STATUS_DISABLED | STATUS_DEPRECATED | STATUS_BLOCKED
    )
}

fn is_valid_execution_mode(mode: &str) -> bool {
    matches!(
        mode,
        EXECUTION_MODE_EMBEDDED_TEST | EXECUTION_MODE_NO_OP | EXECUTION_MODE_DRY_RUN
    )
}

pub fn normalize_wasm_module_descriptor(
    project_id: &str,
    mut input: WasmModuleDescriptor,
) -> WasmModuleDescriptor {
    let now = Utc::now().to_rfc3339();
    input.project_id = project_id.to_string();
    input.updated_at_utc = Some(now.clone());
    if input
        .created_at_utc
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        input.created_at_utc = Some(now);
    }
    input
}

pub fn validate_wasm_module_descriptor(module: &WasmModuleDescriptor) -> Result<(), String> {
    if module.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }
    if module.module_id.trim().is_empty() {
        return Err("module_id must not be empty".to_string());
    }
    if module.module_name.trim().is_empty() {
        return Err("module_name must not be empty".to_string());
    }
    if !is_valid_module_kind(module.module_kind.trim()) {
        return Err(format!("module_kind is invalid: {}", module.module_kind));
    }
    if !is_valid_status(module.status.trim()) {
        return Err(format!("status is invalid: {}", module.status));
    }
    if module.status == STATUS_REGISTERED && module.allowed_functions.is_empty() {
        return Err("registered modules must include at least one allowed function".to_string());
    }

    for function in &module.allowed_functions {
        if function.function_name.trim().is_empty() {
            return Err("function_name must not be empty".to_string());
        }
        for mode in &function.allowed_execution_modes {
            if !is_valid_execution_mode(mode.trim()) {
                return Err(format!("allowed_execution_mode is invalid: {}", mode));
            }
        }
        if function.max_fuel == Some(0) {
            return Err("max_fuel must be positive when present".to_string());
        }
        if function.timeout_ms == Some(0) {
            return Err("timeout_ms must be positive when present".to_string());
        }
    }

    if module.safety.filesystem_access {
        return Err("filesystem_access must be false".to_string());
    }
    if module.safety.network_access {
        return Err("network_access must be false".to_string());
    }
    if module.safety.host_command_access {
        return Err("host_command_access must be false".to_string());
    }
    if module.safety.docker_access {
        return Err("docker_access must be false".to_string());
    }
    if module.safety.artifact_access {
        return Err("artifact_access must be false".to_string());
    }
    if module.safety.installation_task_access {
        return Err("installation_task_access must be false".to_string());
    }
    if module.safety.assisted_execution_access {
        return Err("assisted_execution_access must be false".to_string());
    }
    if module.safety.side_effects_allowed {
        return Err("side_effects_allowed must be false".to_string());
    }
    if module.safety.safe_to_auto_execute {
        return Err("safe_to_auto_execute must be false".to_string());
    }

    Ok(())
}

pub fn default_embedded_add_module_descriptor(project_id: &str) -> WasmModuleDescriptor {
    normalize_wasm_module_descriptor(
        project_id,
        WasmModuleDescriptor {
            project_id: project_id.to_string(),
            module_id: "embedded-add".to_string(),
            module_name: "embedded:add".to_string(),
            module_version: None,
            module_kind: MODULE_KIND_EMBEDDED_TEST.to_string(),
            status: STATUS_REGISTERED.to_string(),
            allowed_functions: vec![WasmFunctionDescriptor {
                function_name: "add".to_string(),
                input_schema: None,
                output_schema: None,
                allowed_execution_modes: vec![EXECUTION_MODE_EMBEDDED_TEST.to_string()],
                max_fuel: Some(1_000_000),
                timeout_ms: Some(1_000),
                metadata: None,
            }],
            artifact_ref: None,
            safety: WasmModuleSafetyMetadata::default(),
            created_at_utc: None,
            updated_at_utc: None,
            metadata: None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_descriptor() -> WasmModuleDescriptor {
        default_embedded_add_module_descriptor("smart-city-campinas")
    }

    #[test]
    fn valid_embedded_test_descriptor_passes() {
        assert!(validate_wasm_module_descriptor(&base_descriptor()).is_ok());
    }

    #[test]
    fn normalization_overrides_project_id() {
        let normalized = normalize_wasm_module_descriptor(
            "project-a",
            WasmModuleDescriptor {
                project_id: "wrong".to_string(),
                module_id: "embedded-add".to_string(),
                module_name: "embedded:add".to_string(),
                module_kind: MODULE_KIND_EMBEDDED_TEST.to_string(),
                status: STATUS_REGISTERED.to_string(),
                ..WasmModuleDescriptor::default()
            },
        );

        assert_eq!(normalized.project_id, "project-a");
        assert!(normalized.created_at_utc.is_some());
        assert!(normalized.updated_at_utc.is_some());
    }

    #[test]
    fn validation_rejects_empty_module_id() {
        let mut descriptor = base_descriptor();
        descriptor.module_id.clear();
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_empty_module_name() {
        let mut descriptor = base_descriptor();
        descriptor.module_name.clear();
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_invalid_module_kind() {
        let mut descriptor = base_descriptor();
        descriptor.module_kind = "explode".to_string();
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_invalid_status() {
        let mut descriptor = base_descriptor();
        descriptor.status = "explode".to_string();
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_empty_function_name() {
        let mut descriptor = base_descriptor();
        descriptor.allowed_functions[0].function_name.clear();
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_invalid_execution_mode() {
        let mut descriptor = base_descriptor();
        descriptor.allowed_functions[0].allowed_execution_modes = vec!["execute".to_string()];
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_max_fuel_zero() {
        let mut descriptor = base_descriptor();
        descriptor.allowed_functions[0].max_fuel = Some(0);
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_timeout_zero() {
        let mut descriptor = base_descriptor();
        descriptor.allowed_functions[0].timeout_ms = Some(0);
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_filesystem_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.filesystem_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_network_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.network_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_host_command_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.host_command_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_docker_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.docker_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_artifact_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.artifact_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_installation_task_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.installation_task_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_assisted_execution_access_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.assisted_execution_access = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_side_effects_allowed_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.side_effects_allowed = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn validation_rejects_safe_to_auto_execute_true() {
        let mut descriptor = base_descriptor();
        descriptor.safety.safe_to_auto_execute = true;
        assert!(validate_wasm_module_descriptor(&descriptor).is_err());
    }

    #[test]
    fn default_embedded_add_descriptor_keeps_all_safety_flags_false() {
        let descriptor = default_embedded_add_module_descriptor("smart-city-campinas");

        assert!(!descriptor.safety.filesystem_access);
        assert!(!descriptor.safety.network_access);
        assert!(!descriptor.safety.host_command_access);
        assert!(!descriptor.safety.docker_access);
        assert!(!descriptor.safety.artifact_access);
        assert!(!descriptor.safety.installation_task_access);
        assert!(!descriptor.safety.assisted_execution_access);
        assert!(!descriptor.safety.side_effects_allowed);
        assert!(!descriptor.safety.safe_to_auto_execute);
    }
}
