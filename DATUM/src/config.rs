use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub agent: AgentConfig,
    pub outputs: OutputsConfig,

    /// Optional collectors config (Phase 1.1+)
    #[serde(default)]
    pub collectors: Option<CollectorsConfig>,
    
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
pub struct HttpConfig {
    pub enabled: bool,
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FileJsonConfig {
    pub enabled: bool,
    pub path: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct CollectorsConfig {
    #[serde(default)]
    pub process_windows: Option<ProcessWindowsConfig>,
    pub process_linux: Option<ProcessLinuxConfig>,
   // pub connection_ping_windows: Option<ConnectionPingWindowsConfig>,
    // pub host_telemetry_windows: Option<HostTelemetryWindowsConfig>,
}
#[derive(Debug, Deserialize, Clone, Default)]
 pub struct ProcessLinuxConfig {
     pub enabled: bool,
     pub mode: Option<String>, // "allowlist" | "top_mem" | "all" | "group_by_name"
     pub names: Option<Vec<String>>,
     pub max_entries: Option<usize>,
 }

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ProcessWindowsConfig {
    pub enabled: bool,
    pub mode: Option<String>, // "allowlist" | "top_mem" | "all"
    pub names: Option<Vec<String>>,
    pub max_entries: Option<usize>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {}", path))?;
    let cfg: Config = toml::from_str(&raw).context("parse TOML config")?;
    Ok(cfg)
}
