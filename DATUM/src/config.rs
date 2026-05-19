use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub agent: AgentConfig,
    pub outputs: OutputsConfig,
    #[serde(default)]
    pub dispatch: Option<DispatchConfig>,

    #[serde(default)]
    pub collectors: Option<CollectorsConfig>,

    #[serde(default)]
    pub services: Option<ServicesConfig>,

    #[serde(default)]
    pub service_registry: Option<ServiceRegistryConfig>,

    #[serde(default)]
    pub auto_recovery: Option<AutoRecoveryConfig>,

    #[serde(default)]
    pub desired_state: Option<DesiredStateConfig>,
}

impl Config {
    pub fn registry_services(&self) -> &[ServiceRegistryService] {
        match &self.service_registry {
            Some(sr) if sr.enabled => &sr.services,
            _ => &[],
        }
    }

    pub fn find_registry_service(&self, name: &str) -> Option<&ServiceRegistryService> {
        self.registry_services()
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn desired_services(&self) -> &[DesiredServiceSpec] {
        match &self.desired_state {
            Some(ds) if ds.enabled => &ds.services,
            _ => &[],
        }
    }

    pub fn find_desired_service(&self, name: &str) -> Option<&DesiredServiceSpec> {
        self.desired_services()
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct AutoRecoveryConfig {
    pub enabled: bool,
    #[serde(default)]
    pub dry_run: Option<bool>,
    #[serde(default)]
    pub allow_services: Option<Vec<String>>,
    #[serde(default)]
    pub cooldown_s: Option<u64>,
    #[serde(default)]
    pub max_attempts_per_window: Option<u32>,
    #[serde(default)]
    pub attempt_window_s: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct DesiredStateConfig {
    pub enabled: bool,
    #[serde(default)]
    pub services: Vec<DesiredServiceSpec>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct DesiredServiceSpec {
    pub name: String,
    pub desired: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OutputsConfig {
    pub file_json: Option<FileJsonConfig>,
    pub http: Option<HttpConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileJsonConfig {
    pub enabled: bool,
    pub path: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HttpConfig {
    pub enabled: bool,
    pub url: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct DispatchConfig {
    #[serde(default)]
    pub enabled: bool,
    pub url: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub timeout_s: Option<u64>,
    #[serde(default)]
    pub execute: Option<bool>,
    #[serde(default)]
    pub publish_results: Option<bool>,
    #[serde(default)]
    pub allow_container_execution: Option<bool>,
    #[serde(default)]
    pub container_mappings: Vec<DispatchContainerMapping>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct DispatchContainerMapping {
    pub module_id: Option<String>,
    pub service_name: Option<String>,
    pub container_name: String,
    pub artifact: Option<String>,
}
#[derive(Debug, Deserialize, Clone, Default)]
pub struct CollectorsConfig {
    #[serde(default)]
    pub docker: Option<DockerConfig>,
    #[serde(default)]
    pub process_windows: Option<ProcessWindowsConfig>,
    #[serde(default)]
    pub process_linux: Option<ProcessLinuxConfig>,
    #[serde(default)]
    pub connection_ping: Option<ConnectionPingConfig>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct DockerConfig {
    pub enabled: bool,
    pub mode: Option<String>,
    pub names: Option<Vec<String>>,
    pub images: Option<Vec<String>>,
    pub max_entries: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProcessWindowsConfig {
    pub enabled: bool,
    pub mode: Option<String>,
    pub names: Option<Vec<String>>,
    pub max_entries: Option<usize>,
    #[serde(default)]
    pub include_pids: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProcessLinuxConfig {
    pub enabled: bool,
    pub mode: Option<String>,
    pub names: Option<Vec<String>>,
    pub max_entries: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ConnectionPingConfig {
    pub enabled: bool,
    pub targets: Option<Vec<String>>,
    pub count: Option<u32>,
    pub timeout_s: Option<u32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    pub node_id: String,
    pub cycle_s: u64,
    pub schema_version: String,
    pub hostname: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub events: Option<AgentEventsConfig>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct AgentEventsConfig {
    pub max_per_snapshot: Option<usize>,
    pub deduplicate: Option<bool>,
    pub prefer_severity: Option<bool>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServicesConfig {
    pub enabled: bool,
    #[serde(default)]
    pub items: Vec<ServiceItem>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceCheckSpec {
    pub kind: String,
    #[serde(flatten)]
    pub params: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceDependencySpec {
    pub kind: String,
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServiceItem {
    pub name: String,
    #[serde(default)]
    pub implemented_by: Vec<ImplementedBy>,
    #[serde(default)]
    pub checks: Vec<ServiceCheckSpec>,
    #[serde(default)]
    pub depends_on: Vec<ServiceDependencySpec>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ImplementedBy {
    pub container_name: Option<String>,
    pub process_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceRegistryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub services: Vec<ServiceRegistryService>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceRegistryService {
    pub name: String,
    pub runtime: String,
    #[serde(default)]
    pub selector: Option<ServiceSelector>,
    #[serde(default)]
    pub recovery: Option<ServiceRecoverySpec>,
    #[serde(default)]
    pub checks: Vec<ServiceCheckSpec>,
    #[serde(default)]
    pub depends_on: Vec<ServiceDependencySpec>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceSelector {
    #[serde(default)]
    pub container_name: Option<String>,
    #[serde(default)]
    pub process_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceRecoverySpec {
    pub policy: String,
    #[serde(default)]
    pub command_windows: Option<String>,
    #[serde(default)]
    pub command_linux: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub shell: Option<bool>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {}", path))?;
    let cfg: Config = toml::from_str(&raw).context("parse TOML config")?;
    Ok(cfg)
}
