use std::process::Command;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::{
    config::{Config, DispatchContainerMapping},
    dispatch::model::DispatchAction,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerExecutionOutcome {
    AlreadyRunning { container_name: String },
    Started { container_name: String },
    NotInstalled { reason: String },
    MissingLocalMapping { reason: String },
    DockerUnavailable { reason: String },
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerPresence {
    Running,
    Stopped,
    Missing,
}

pub trait ContainerRuntime {
    fn inspect_container(&self, container_name: &str) -> Result<ContainerPresence>;
    fn start_container(&self, container_name: &str) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct DockerCliRuntime;

impl ContainerRuntime for DockerCliRuntime {
    fn inspect_container(&self, container_name: &str) -> Result<ContainerPresence> {
        let output = Command::new("docker")
            .args(["inspect", "-f", "{{.State.Running}}", container_name])
            .output()
            .with_context(|| {
                format!("failed to execute docker inspect for container '{container_name}'")
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
            .trim()
            .to_string();

            if combined.contains("No such object") || combined.contains("No such container") {
                return Ok(ContainerPresence::Missing);
            }

            return Err(anyhow!(if stderr.is_empty() {
                "docker inspect failed".to_string()
            } else {
                stderr
            }));
        }

        let stdout = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();
        match stdout.as_str() {
            "true" => Ok(ContainerPresence::Running),
            "false" => Ok(ContainerPresence::Stopped),
            other => Err(anyhow!("unexpected docker inspect state output: {other}")),
        }
    }

    fn start_container(&self, container_name: &str) -> Result<()> {
        let output = Command::new("docker")
            .args(["start", container_name])
            .output()
            .with_context(|| {
                format!("failed to execute docker start for container '{container_name}'")
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(anyhow!(if stderr.is_empty() {
                "docker start failed".to_string()
            } else {
                stderr
            }));
        }

        Ok(())
    }
}

pub fn ensure_container_running_guarded(
    action: &DispatchAction,
    local_node_id: &str,
    config: &Config,
) -> ContainerExecutionOutcome {
    let runtime = DockerCliRuntime;
    ensure_container_running_guarded_with_runtime(action, local_node_id, config, &runtime)
}

pub fn ensure_container_running_guarded_with_runtime(
    action: &DispatchAction,
    local_node_id: &str,
    config: &Config,
    runtime: &dyn ContainerRuntime,
) -> ContainerExecutionOutcome {
    let container_name = match resolve_trusted_container_mapping(action, local_node_id, config) {
        Ok(name) => name,
        Err(outcome) => return outcome,
    };

    let presence = match runtime.inspect_container(&container_name) {
        Ok(presence) => presence,
        Err(err) => {
            return ContainerExecutionOutcome::DockerUnavailable {
                reason: format!("Docker is not available or daemon is unreachable: {err}"),
            }
        }
    };

    match presence {
        ContainerPresence::Running => ContainerExecutionOutcome::AlreadyRunning { container_name },
        ContainerPresence::Missing => ContainerExecutionOutcome::NotInstalled {
            reason: format!(
                "trusted container mapping exists, but container is not installed locally: {container_name}"
            ),
        },
        ContainerPresence::Stopped => match runtime.start_container(&container_name) {
            Ok(()) => ContainerExecutionOutcome::Started { container_name },
            Err(err) => ContainerExecutionOutcome::Failed {
                reason: format!("failed to start existing container '{container_name}': {err}"),
            },
        },
    }
}

fn resolve_trusted_container_mapping(
    action: &DispatchAction,
    local_node_id: &str,
    config: &Config,
) -> Result<String, ContainerExecutionOutcome> {
    let candidates = action_candidates(action);

    for service in config.registry_services() {
        if !service.runtime.eq_ignore_ascii_case("container") {
            continue;
        }

        let container_name = service
            .selector
            .as_ref()
            .and_then(|selector| selector.container_name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty());

        let Some(container_name) = container_name else {
            continue;
        };

        if candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(&service.name))
            || candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(container_name))
        {
            return Ok(container_name.to_string());
        }
    }

    if let Some(dispatch_cfg) = config.dispatch.as_ref() {
        for mapping in &dispatch_cfg.container_mappings {
            if !mapping_matches(mapping, &candidates) {
                continue;
            }

            if mapping.container_name.trim().is_empty() {
                continue;
            }

            return Ok(mapping.container_name.trim().to_string());
        }
    }

    let module_hint = candidates
        .first()
        .cloned()
        .unwrap_or_else(|| "<unknown>".to_string());
    Err(ContainerExecutionOutcome::MissingLocalMapping {
        reason: format!(
            "no trusted local container mapping found for node {local_node_id} and action hint {module_hint}"
        ),
    })
}

fn mapping_matches(mapping: &DispatchContainerMapping, candidates: &[String]) -> bool {
    [
        mapping.module_id.as_deref(),
        mapping.service_name.as_deref(),
        mapping.artifact.as_deref(),
        Some(mapping.container_name.as_str()),
    ]
    .into_iter()
    .flatten()
    .any(|value| {
        candidates
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(value))
    })
}

fn action_candidates(action: &DispatchAction) -> Vec<String> {
    let mut out = Vec::new();

    if let Some(container_name) = ai_context_string(action.ai_context.as_ref(), "container_name") {
        out.push(container_name);
    }
    if let Some(service_name) = ai_context_string(action.ai_context.as_ref(), "service_name") {
        out.push(service_name);
    }
    if let Some(module_id) = ai_context_string(action.ai_context.as_ref(), "module_id") {
        out.push(module_id);
    }
    if let Some(artifact) = action
        .artifact
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        out.push(artifact.to_string());
    }

    out
}

fn ai_context_string(value: Option<&Value>, key: &str) -> Option<String> {
    value?
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_container_running_guarded_with_runtime, ContainerExecutionOutcome,
        ContainerPresence, ContainerRuntime,
    };
    use crate::{
        config::{
            AgentConfig, Config, DispatchConfig, DispatchContainerMapping, OutputsConfig,
            ServiceRegistryConfig, ServiceRegistryService, ServiceSelector,
        },
        dispatch::model::DispatchAction,
    };
    use std::cell::RefCell;

    struct FakeRuntime {
        inspect_result: std::result::Result<ContainerPresence, String>,
        start_result: std::result::Result<(), String>,
        inspect_calls: RefCell<Vec<String>>,
        start_calls: RefCell<Vec<String>>,
    }

    impl FakeRuntime {
        fn new(
            inspect_result: std::result::Result<ContainerPresence, String>,
            start_result: std::result::Result<(), String>,
        ) -> Self {
            Self {
                inspect_result,
                start_result,
                inspect_calls: RefCell::new(Vec::new()),
                start_calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl ContainerRuntime for FakeRuntime {
        fn inspect_container(&self, container_name: &str) -> anyhow::Result<ContainerPresence> {
            self.inspect_calls
                .borrow_mut()
                .push(container_name.to_string());
            self.inspect_result.clone().map_err(anyhow::Error::msg)
        }

        fn start_container(&self, container_name: &str) -> anyhow::Result<()> {
            self.start_calls
                .borrow_mut()
                .push(container_name.to_string());
            self.start_result.clone().map_err(anyhow::Error::msg)
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
                enabled: false,
                url: None,
                project_id: None,
                timeout_s: None,
                execute: None,
                publish_results: None,
                allow_container_execution: Some(true),
                container_mappings: Vec::new(),
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
                    ..ServiceRegistryService::default()
                }],
            }),
            auto_recovery: None,
            desired_state: None,
        }
    }

    fn action() -> DispatchAction {
        DispatchAction {
            action_id: Some("gap:1".to_string()),
            action_kind: Some("ensure_container_running".to_string()),
            runtime: Some("container".to_string()),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
            ai_context: Some(serde_json::json!({
                "module_id": "chirpstack-primary",
                "service_name": "chirpstack-primary"
            })),
            ..DispatchAction::default()
        }
    }

    #[test]
    fn uses_service_registry_mapping_when_available() {
        let cfg = base_config();
        let runtime = FakeRuntime::new(Ok(ContainerPresence::Running), Ok(()));

        let outcome =
            ensure_container_running_guarded_with_runtime(&action(), "fog-01", &cfg, &runtime);

        assert_eq!(
            outcome,
            ContainerExecutionOutcome::AlreadyRunning {
                container_name: "chirpstack".to_string()
            }
        );
    }

    #[test]
    fn returns_missing_local_mapping_when_no_mapping_exists() {
        let mut cfg = base_config();
        cfg.service_registry = None;
        let runtime = FakeRuntime::new(Ok(ContainerPresence::Running), Ok(()));

        let outcome =
            ensure_container_running_guarded_with_runtime(&action(), "fog-01", &cfg, &runtime);

        assert!(matches!(
            outcome,
            ContainerExecutionOutcome::MissingLocalMapping { .. }
        ));
    }

    #[test]
    fn supports_optional_dispatch_container_mapping() {
        let mut cfg = base_config();
        cfg.service_registry = None;
        cfg.dispatch
            .as_mut()
            .expect("dispatch config")
            .container_mappings = vec![DispatchContainerMapping {
            module_id: Some("chirpstack-primary".to_string()),
            service_name: None,
            container_name: "chirpstack".to_string(),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
        }];
        let runtime = FakeRuntime::new(Ok(ContainerPresence::Running), Ok(()));

        let outcome =
            ensure_container_running_guarded_with_runtime(&action(), "fog-01", &cfg, &runtime);

        assert!(matches!(
            outcome,
            ContainerExecutionOutcome::AlreadyRunning { .. }
        ));
    }

    #[test]
    fn implementation_never_uses_docker_run_pull_or_rm() {
        let source = include_str!("container_executor.rs");
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
