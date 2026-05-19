use std::process::Command;

use anyhow::{Context, Result};
use serde_json::json;

use crate::{
    capabilities::model::{
        ContainerCapabilities, FlowCapabilities, NodeCapabilities, ProcessCapabilities,
        RuntimeCapabilities, WasmCapabilities,
    },
    config::{Config, ServiceRecoverySpec},
};

pub trait DockerCapabilityDetector {
    fn docker_available(&self) -> Result<bool>;
}

#[derive(Debug, Default)]
pub struct DockerCliCapabilityDetector;

impl DockerCapabilityDetector for DockerCliCapabilityDetector {
    fn docker_available(&self) -> Result<bool> {
        let output = Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .output()
            .context("failed to execute docker version")?;

        Ok(output.status.success())
    }
}

pub fn build_node_capabilities(
    config: &Config,
    node_id: &str,
    project_id: &str,
) -> NodeCapabilities {
    let detector = DockerCliCapabilityDetector;
    build_node_capabilities_with_detector(config, node_id, project_id, &detector)
}

pub fn build_node_capabilities_with_detector(
    config: &Config,
    node_id: &str,
    project_id: &str,
    detector: &dyn DockerCapabilityDetector,
) -> NodeCapabilities {
    let stage = config
        .agent
        .stage
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    let guarded_container_support = has_guarded_container_support(config);
    let docker_available = detector.docker_available().unwrap_or(false);
    let container_available = guarded_container_support && docker_available;
    let allow_container_execution = config
        .dispatch
        .as_ref()
        .and_then(|dispatch| dispatch.allow_container_execution)
        .unwrap_or(false);

    let container = Some(ContainerCapabilities {
        available: container_available,
        engine: if docker_available {
            Some("docker".to_string())
        } else {
            None
        },
        allow_execution: Some(allow_container_execution),
        supports_existing_start: Some(container_available),
        supports_restart: Some(false),
        supports_create: Some(false),
        supports_pull: Some(false),
        ..ContainerCapabilities::default()
    });

    let process_restart_available = has_restart_command_support(config);
    let process = Some(ProcessCapabilities {
        available: process_restart_available,
        allow_execution: Some(false),
        supports_restart_command: Some(process_restart_available),
        ..ProcessCapabilities::default()
    });

    NodeCapabilities {
        project_id: Some(project_id.to_string()),
        node_id: Some(node_id.to_string()),
        stage: Some(stage),
        updated_at_utc: None,
        source: Some("client".to_string()),
        runtimes: RuntimeCapabilities {
            container,
            wasm: Some(WasmCapabilities {
                available: false,
                runtime: None,
                allow_execution: Some(false),
                supports_wasi: Some(false),
                supports_http: Some(false),
                supports_mqtt: Some(false),
                ..WasmCapabilities::default()
            }),
            process,
            ..RuntimeCapabilities::default()
        },
        flow: Some(FlowCapabilities {
            supports_mqtt_reconfigure: Some(false),
            supports_http_reconfigure: Some(false),
            supports_bridge_reconfigure: Some(false),
            supports_proxy_reconfigure: Some(false),
            ..FlowCapabilities::default()
        }),
        resources: Some(json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH
        })),
        metadata: Some(json!({
            "client_version": env!("CARGO_PKG_VERSION"),
            "capability_schema_version": "0.1.0"
        })),
        ..NodeCapabilities::default()
    }
}

fn has_guarded_container_support(config: &Config) -> bool {
    config.registry_services().iter().any(|service| {
        service.runtime.eq_ignore_ascii_case("container")
            && service
                .selector
                .as_ref()
                .and_then(|selector| selector.container_name.as_deref())
                .map(str::trim)
                .is_some_and(|name| !name.is_empty())
    }) || config
        .dispatch
        .as_ref()
        .map(|dispatch| {
            dispatch
                .container_mappings
                .iter()
                .any(|mapping| !mapping.container_name.trim().is_empty())
        })
        .unwrap_or(false)
}

fn has_restart_command_support(config: &Config) -> bool {
    config.registry_services().iter().any(|service| {
        let Some(recovery) = service.recovery.as_ref() else {
            return false;
        };

        recovery.policy.eq_ignore_ascii_case("restart_command")
            && platform_command(recovery)
                .map(str::trim)
                .is_some_and(|cmd| !cmd.is_empty())
    })
}

fn platform_command(recovery: &ServiceRecoverySpec) -> Option<&str> {
    #[cfg(target_os = "windows")]
    {
        recovery
            .command_windows
            .as_deref()
            .or(recovery.command_linux.as_deref())
    }

    #[cfg(not(target_os = "windows"))]
    {
        recovery
            .command_linux
            .as_deref()
            .or(recovery.command_windows.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::{build_node_capabilities_with_detector, DockerCapabilityDetector};
    use crate::config::{
        AgentConfig, Config, DispatchConfig, DispatchContainerMapping, OutputsConfig,
        ServiceRecoverySpec, ServiceRegistryConfig, ServiceRegistryService, ServiceSelector,
    };

    struct FakeDetector {
        available: bool,
    }

    impl DockerCapabilityDetector for FakeDetector {
        fn docker_available(&self) -> anyhow::Result<bool> {
            Ok(self.available)
        }
    }

    fn base_config() -> Config {
        Config {
            agent: AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "0.0.1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: OutputsConfig {
                file_json: None,
                http: None,
            },
            dispatch: Some(DispatchConfig {
                allow_container_execution: Some(false),
                container_mappings: vec![DispatchContainerMapping {
                    module_id: Some("chirpstack-primary".to_string()),
                    service_name: None,
                    container_name: "chirpstack".to_string(),
                    artifact: Some("chirpstack/chirpstack:latest".to_string()),
                }],
                ..DispatchConfig::default()
            }),
            collectors: None,
            services: None,
            service_registry: Some(ServiceRegistryConfig {
                enabled: true,
                services: vec![ServiceRegistryService {
                    name: "chirpstack-primary".to_string(),
                    runtime: "container".to_string(),
                    selector: Some(ServiceSelector {
                        container_name: Some("chirpstack".to_string()),
                        process_name: None,
                    }),
                    recovery: Some(ServiceRecoverySpec {
                        policy: "restart_command".to_string(),
                        command_windows: Some("py -m http.server 8001".to_string()),
                        command_linux: Some("python3 -m http.server 8001".to_string()),
                        working_dir: Some(".".to_string()),
                        shell: Some(false),
                    }),
                    ..ServiceRegistryService::default()
                }],
            }),
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn builder_uses_project_and_node_id() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );

        assert_eq!(caps.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(caps.node_id.as_deref(), Some("fog-01"));
    }

    #[test]
    fn missing_stage_defaults_to_unknown() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );

        assert_eq!(caps.stage.as_deref(), Some("unknown"));
    }

    #[test]
    fn container_allow_execution_reflects_dispatch_config() {
        let mut cfg = base_config();
        cfg.dispatch.as_mut().unwrap().allow_container_execution = Some(true);

        let caps = build_node_capabilities_with_detector(
            &cfg,
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );

        assert_eq!(
            caps.runtimes
                .container
                .as_ref()
                .and_then(|container| container.allow_execution),
            Some(true)
        );
    }

    #[test]
    fn supports_existing_start_true_only_when_guarded_support_available() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );
        assert_eq!(
            caps.runtimes
                .container
                .as_ref()
                .and_then(|container| container.supports_existing_start),
            Some(true)
        );

        let mut cfg = base_config();
        cfg.service_registry = None;
        cfg.dispatch.as_mut().unwrap().container_mappings.clear();

        let caps = build_node_capabilities_with_detector(
            &cfg,
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );
        assert_eq!(
            caps.runtimes
                .container
                .as_ref()
                .and_then(|container| container.supports_existing_start),
            Some(false)
        );
    }

    #[test]
    fn restart_create_pull_remain_false() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );
        let container = caps.runtimes.container.as_ref().unwrap();

        assert_eq!(container.supports_restart, Some(false));
        assert_eq!(container.supports_create, Some(false));
        assert_eq!(container.supports_pull, Some(false));
    }

    #[test]
    fn wasm_capability_is_false() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );
        let wasm = caps.runtimes.wasm.as_ref().unwrap();

        assert!(!wasm.available);
        assert_eq!(wasm.allow_execution, Some(false));
    }

    #[test]
    fn process_capability_is_conservative() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );
        let process = caps.runtimes.process.as_ref().unwrap();

        assert!(process.available);
        assert_eq!(process.allow_execution, Some(false));
        assert_eq!(process.supports_restart_command, Some(true));
    }

    #[test]
    fn docker_available_sets_engine_and_availability() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: true },
        );
        let container = caps.runtimes.container.as_ref().unwrap();

        assert!(container.available);
        assert_eq!(container.engine.as_deref(), Some("docker"));
    }

    #[test]
    fn docker_unavailable_keeps_container_capability_conservative() {
        let caps = build_node_capabilities_with_detector(
            &base_config(),
            "fog-01",
            "smart-city-campinas",
            &FakeDetector { available: false },
        );
        let container = caps.runtimes.container.as_ref().unwrap();

        assert!(!container.available);
        assert_eq!(container.engine, None);
    }

    #[test]
    fn implementation_never_uses_docker_mutating_commands() {
        let source = include_str!("builder.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);
        let docker_run = ["docker", "run"].join(" ");
        let run_arg = format!("\"{}\"", "run");
        let pull_arg = format!("\"{}\"", "pull");
        let rm_arg = format!("\"{}\"", "rm");

        assert!(!implementation.contains(&docker_run));
        assert!(!implementation.contains(&run_arg));
        assert!(!implementation.contains(&pull_arg));
        assert!(!implementation.contains(&rm_arg));
    }
}
