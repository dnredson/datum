use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmModuleDescriptor {
    pub project_id: Option<String>,
    pub module_id: Option<String>,
    pub module_name: Option<String>,
    pub module_version: Option<String>,
    pub module_kind: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub allowed_functions: Vec<WasmFunctionDescriptor>,
    pub artifact_ref: Option<WasmArtifactRef>,
    pub safety: Option<WasmModuleSafetyMetadata>,
    pub created_at_utc: Option<String>,
    pub updated_at_utc: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmFunctionDescriptor {
    pub function_name: Option<String>,
    pub input_schema: Option<serde_json::Value>,
    pub output_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub allowed_execution_modes: Vec<String>,
    pub max_fuel: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmArtifactRef {
    pub artifact_id: Option<String>,
    pub artifact_uri: Option<String>,
    pub sha256: Option<String>,
    pub signature_ref: Option<String>,
    pub provenance: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmModuleSafetyMetadata {
    pub filesystem_access: Option<bool>,
    pub network_access: Option<bool>,
    pub host_command_access: Option<bool>,
    pub docker_access: Option<bool>,
    pub artifact_access: Option<bool>,
    pub installation_task_access: Option<bool>,
    pub assisted_execution_access: Option<bool>,
    pub side_effects_allowed: Option<bool>,
    pub safe_to_auto_execute: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmModuleDescriptorList {
    pub project_id: Option<String>,
    pub count: Option<u64>,
    #[serde(default)]
    pub modules: Vec<WasmModuleDescriptor>,
}

#[cfg(test)]
mod tests {
    use super::{WasmModuleDescriptor, WasmModuleDescriptorList};

    #[test]
    fn parses_wasm_module_descriptor() {
        let parsed: WasmModuleDescriptor = serde_json::from_value(serde_json::json!({
            "project_id": "smart-city-campinas",
            "module_id": "embedded-add",
            "module_name": "embedded:add",
            "module_version": "v0",
            "module_kind": "embedded_test",
            "status": "registered",
            "allowed_functions": [
                {
                    "function_name": "add",
                    "allowed_execution_modes": ["embedded_test"],
                    "max_fuel": 1000000,
                    "timeout_ms": 1000
                }
            ],
            "artifact_ref": {
                "artifact_uri": "opaque://metadata-only",
                "sha256": "abc123"
            },
            "safety": {
                "filesystem_access": false,
                "network_access": false,
                "host_command_access": false,
                "docker_access": false,
                "artifact_access": false,
                "installation_task_access": false,
                "assisted_execution_access": false,
                "side_effects_allowed": false,
                "safe_to_auto_execute": false
            },
            "metadata": {
                "note": "metadata only"
            },
            "extra_field": "tolerated"
        }))
        .expect("descriptor should parse");

        assert_eq!(parsed.module_id.as_deref(), Some("embedded-add"));
        assert_eq!(parsed.allowed_functions.len(), 1);
        assert_eq!(
            parsed
                .artifact_ref
                .as_ref()
                .and_then(|artifact| artifact.artifact_uri.as_deref()),
            Some("opaque://metadata-only")
        );
    }

    #[test]
    fn parses_wasm_module_descriptor_list() {
        let parsed: WasmModuleDescriptorList = serde_json::from_value(serde_json::json!({
            "project_id": "smart-city-campinas",
            "count": 1,
            "modules": [
                {
                    "module_id": "embedded-add",
                    "module_name": "embedded:add",
                    "allowed_functions": []
                }
            ]
        }))
        .expect("descriptor list should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.count, Some(1));
        assert_eq!(parsed.modules.len(), 1);
    }

    #[test]
    fn parses_all_false_safety_metadata() {
        let parsed: WasmModuleDescriptor = serde_json::from_value(serde_json::json!({
            "module_id": "embedded-add",
            "module_name": "embedded:add",
            "allowed_functions": [],
            "safety": {
                "filesystem_access": false,
                "network_access": false,
                "host_command_access": false,
                "docker_access": false,
                "artifact_access": false,
                "installation_task_access": false,
                "assisted_execution_access": false,
                "side_effects_allowed": false,
                "safe_to_auto_execute": false
            }
        }))
        .expect("descriptor should parse");

        let safety = parsed.safety.expect("safety metadata should exist");
        assert_eq!(safety.filesystem_access, Some(false));
        assert_eq!(safety.network_access, Some(false));
        assert_eq!(safety.host_command_access, Some(false));
        assert_eq!(safety.docker_access, Some(false));
        assert_eq!(safety.artifact_access, Some(false));
        assert_eq!(safety.installation_task_access, Some(false));
        assert_eq!(safety.assisted_execution_access, Some(false));
        assert_eq!(safety.side_effects_allowed, Some(false));
        assert_eq!(safety.safe_to_auto_execute, Some(false));
    }
}
