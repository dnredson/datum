use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub agent: AgentConfig,
    pub outputs: OutputsConfig,

    #[serde(default)]
    pub collectors: Option<CollectorsConfig>,

    // =========================
    // Services (Fase 2.1)
    // =========================
    #[serde(default)]
    pub services: Option<ServicesConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    pub node_id: String,
    pub cycle_s: u64,
    pub schema_version: String,

    /// opcional, só para fixar host_id em cenários de teste
    pub hostname: Option<String>,
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
}

// =========================
// Collectors
// =========================

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
    pub mode: Option<String>, // "allowlist" | "all" | "top" (futuro)
    pub names: Option<Vec<String>>,  // nomes de containers (ex: ["teste-monitoramento"])
    pub images: Option<Vec<String>>, // imagens (ex: ["busybox", "nginx:latest"])
    pub max_entries: Option<usize>,  // limita quantidade retornada (após filtro)
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProcessWindowsConfig {
    pub enabled: bool,
    pub mode: Option<String>,       // "allowlist" | "top_mem" | "all" | "group_by_name"
    pub names: Option<Vec<String>>, // em Windows costuma ser "chrome.exe"
    pub max_entries: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProcessLinuxConfig {
    pub enabled: bool,
    pub mode: Option<String>,       // "allowlist" | "top_mem" | "all" | "group_by_name"
    pub names: Option<Vec<String>>, // em Linux: "sshd", "mosquitto"
    pub max_entries: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ConnectionPingConfig {
    pub enabled: bool,
    pub targets: Option<Vec<String>>,
    pub count: Option<u32>,
    pub timeout_s: Option<u32>,
}

// =========================
// Services (Fase 2.1)
// =========================

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServicesConfig {
    pub enabled: bool,

    #[serde(default)]
    pub items: Vec<ServiceItem>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceCheckSpec {
    pub kind: String,               // "http_health" | "tcp_port" | "mqtt_pubsub"
    #[serde(flatten)]
    pub params: std::collections::BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ServiceDependencySpec {
    pub kind: String,               // "connection"
    pub target: Option<String>,     // "1.1.1.1"
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

pub fn load_config(path: &str) -> Result<Config> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {}", path))?;
    let cfg: Config = toml::from_str(&raw).context("parse TOML config")?;
    Ok(cfg)
}