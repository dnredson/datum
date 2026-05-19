use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeCapabilities {
    pub project_id: String,
    pub node_id: String,
    #[serde(default)]
    pub stage: Option<String>,
    pub updated_at_utc: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityDecision {
    Supported,
    Unsupported { reason: String },
    Unknown { reason: String },
}

pub fn normalize_node_capabilities(mut input: NodeCapabilities) -> NodeCapabilities {
    if input
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        input.source = Some("manual".to_string());
    }

    if input
        .stage
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        input.stage = Some("unknown".to_string());
    }

    input
}

pub fn validate_node_capabilities(input: &NodeCapabilities) -> Result<(), String> {
    if input.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }

    if input.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    let stage = input.stage.as_deref().unwrap_or("unknown");
    if !matches!(
        stage,
        "thing" | "mist" | "fog" | "cloudlet" | "cloud" | "app" | "unknown"
    ) {
        return Err(format!("invalid stage: {stage}"));
    }

    if input.runtimes.container.is_none()
        && input.runtimes.wasm.is_none()
        && input.runtimes.process.is_none()
    {
        return Err("at least one runtime capability must be present".to_string());
    }

    if let Some(container) = &input.runtimes.container {
        if !container.available
            && [
                container.allow_execution,
                container.supports_existing_start,
                container.supports_restart,
                container.supports_create,
                container.supports_pull,
            ]
            .into_iter()
            .flatten()
            .any(|v| v)
        {
            return Err("container.available=false cannot imply execution support".to_string());
        }
    }

    if let Some(wasm) = &input.runtimes.wasm {
        if !wasm.available
            && [
                wasm.allow_execution,
                wasm.supports_wasi,
                wasm.supports_http,
                wasm.supports_mqtt,
            ]
            .into_iter()
            .flatten()
            .any(|v| v)
        {
            return Err("wasm.available=false cannot imply execution support".to_string());
        }
    }

    Ok(())
}

pub fn node_supports_action(
    capabilities: &NodeCapabilities,
    action_kind: &str,
    runtime: &str,
) -> CapabilityDecision {
    if action_kind.starts_with("migrate_") {
        return CapabilityDecision::Unknown {
            reason: "migration capability planning is not implemented yet".to_string(),
        };
    }

    match (action_kind, runtime) {
        ("ensure_container_running", "container") => {
            let Some(container) = &capabilities.runtimes.container else {
                return CapabilityDecision::Unsupported {
                    reason: "container runtime capability is not registered".to_string(),
                };
            };

            if !container.available {
                return CapabilityDecision::Unsupported {
                    reason: "container runtime is unavailable".to_string(),
                };
            }

            if !container.allow_execution.unwrap_or(false) {
                return CapabilityDecision::Unsupported {
                    reason: "container execution is not allowed".to_string(),
                };
            }

            if !container.supports_existing_start.unwrap_or(false) {
                return CapabilityDecision::Unsupported {
                    reason: "container runtime does not support starting existing containers"
                        .to_string(),
                };
            }

            CapabilityDecision::Supported
        }
        ("ensure_wasm_running", "wasm") => {
            let Some(wasm) = &capabilities.runtimes.wasm else {
                return CapabilityDecision::Unsupported {
                    reason: "wasm runtime capability is not registered".to_string(),
                };
            };

            if !wasm.available {
                return CapabilityDecision::Unsupported {
                    reason: "wasm runtime is unavailable".to_string(),
                };
            }

            if !wasm.allow_execution.unwrap_or(false) {
                return CapabilityDecision::Unsupported {
                    reason: "wasm execution is not allowed".to_string(),
                };
            }

            CapabilityDecision::Supported
        }
        _ => CapabilityDecision::Unknown {
            reason: format!(
                "capability decision is unknown for action_kind '{}' and runtime '{}'",
                action_kind, runtime
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_caps() -> NodeCapabilities {
        NodeCapabilities {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            stage: Some("fog".to_string()),
            updated_at_utc: "2026-05-19T13:00:00Z".to_string(),
            source: Some("manual".to_string()),
            runtimes: RuntimeCapabilities {
                container: Some(ContainerCapabilities {
                    available: true,
                    engine: Some("docker".to_string()),
                    allow_execution: Some(true),
                    supports_existing_start: Some(true),
                    supports_restart: Some(false),
                    supports_create: Some(false),
                    supports_pull: Some(false),
                    ..ContainerCapabilities::default()
                }),
                ..RuntimeCapabilities::default()
            },
            ..NodeCapabilities::default()
        }
    }

    #[test]
    fn valid_container_capability_passes_validation() {
        assert!(validate_node_capabilities(&base_caps()).is_ok());
    }

    #[test]
    fn missing_project_id_fails() {
        let mut caps = base_caps();
        caps.project_id.clear();
        assert!(validate_node_capabilities(&caps).is_err());
    }

    #[test]
    fn missing_node_id_fails() {
        let mut caps = base_caps();
        caps.node_id.clear();
        assert!(validate_node_capabilities(&caps).is_err());
    }

    #[test]
    fn invalid_stage_fails() {
        let mut caps = base_caps();
        caps.stage = Some("planetary".to_string());
        assert!(validate_node_capabilities(&caps).is_err());
    }

    #[test]
    fn normalization_fills_source_and_stage() {
        let caps = normalize_node_capabilities(NodeCapabilities {
            project_id: "p".to_string(),
            node_id: "n".to_string(),
            updated_at_utc: "2026-05-19T13:00:00Z".to_string(),
            runtimes: RuntimeCapabilities {
                process: Some(ProcessCapabilities {
                    available: true,
                    allow_execution: Some(false),
                    supports_restart_command: Some(false),
                    ..ProcessCapabilities::default()
                }),
                ..RuntimeCapabilities::default()
            },
            ..NodeCapabilities::default()
        });

        assert_eq!(caps.source.as_deref(), Some("manual"));
        assert_eq!(caps.stage.as_deref(), Some("unknown"));
    }

    #[test]
    fn ensure_container_running_supported_when_flags_match() {
        assert_eq!(
            node_supports_action(&base_caps(), "ensure_container_running", "container"),
            CapabilityDecision::Supported
        );
    }

    #[test]
    fn ensure_container_running_unsupported_when_container_unavailable() {
        let mut caps = base_caps();
        caps.runtimes.container.as_mut().unwrap().available = false;
        assert!(matches!(
            node_supports_action(&caps, "ensure_container_running", "container"),
            CapabilityDecision::Unsupported { .. }
        ));
    }

    #[test]
    fn ensure_container_running_unsupported_when_execution_not_allowed() {
        let mut caps = base_caps();
        caps.runtimes.container.as_mut().unwrap().allow_execution = Some(false);
        assert!(matches!(
            node_supports_action(&caps, "ensure_container_running", "container"),
            CapabilityDecision::Unsupported { .. }
        ));
    }

    #[test]
    fn ensure_wasm_running_supported_only_when_available_and_allowed() {
        let mut caps = base_caps();
        caps.runtimes.wasm = Some(WasmCapabilities {
            available: true,
            allow_execution: Some(true),
            ..WasmCapabilities::default()
        });

        assert_eq!(
            node_supports_action(&caps, "ensure_wasm_running", "wasm"),
            CapabilityDecision::Supported
        );
    }

    #[test]
    fn migrate_returns_unknown_with_not_implemented_reason() {
        assert!(matches!(
            node_supports_action(&base_caps(), "migrate_container", "container"),
            CapabilityDecision::Unknown { reason }
            if reason.contains("migration capability planning is not implemented yet")
        ));
    }

    #[test]
    fn unknown_action_kind_returns_unknown() {
        assert!(matches!(
            node_supports_action(&base_caps(), "reboot_everything", "container"),
            CapabilityDecision::Unknown { .. }
        ));
    }
}
