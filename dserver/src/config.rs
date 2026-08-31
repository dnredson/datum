use std::{fs, path::Path};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub firebase: FirebaseConfig,
    pub cors: Option<CorsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub bind: Option<String>,
    pub debug: Option<bool>,
    #[serde(default)]
    pub controlled_wasm_state_path: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CorsConfig {
    pub enabled: Option<bool>,
    pub allow_any_origin: Option<bool>,
    pub allowed_origins: Option<Vec<String>>,
    pub allowed_methods: Option<Vec<String>>,
    pub allowed_headers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FirebaseConfig {
    pub project_id: String,
    pub credentials_path: String,
    pub collection_raw: Option<String>,
    pub collection_latest: Option<String>,
    pub collection_nodes: Option<String>,
    pub collection_graph_latest: Option<String>,
    pub collection_analysis_latest: Option<String>,
    pub collection_global_events_latest: Option<String>,
    pub collection_project_topology: Option<String>,
    pub collection_project_dmap: Option<String>,
    pub collection_action_plan_latest: Option<String>,
    pub collection_dispatch_latest: Option<String>,
    pub collection_node_capabilities: Option<String>,
    pub collection_action_results: Option<String>,
    pub collection_action_results_latest: Option<String>,
    pub collection_wasm_runtime_reports: Option<String>,
    pub collection_wasm_runtime_reports_latest: Option<String>,
    pub collection_wasm_module_registry: Option<String>,
    pub collection_local_mapping_preflight_reports: Option<String>,
    pub collection_local_mapping_preflight_reports_latest: Option<String>,
    pub collection_local_dgraph_snapshots: Option<String>,
    pub collection_local_dgraph_snapshots_latest: Option<String>,
    pub collection_installation_tasks: Option<String>,
    pub collection_installation_task_approvals: Option<String>,
    pub collection_installation_task_approvals_latest: Option<String>,
    pub collection_assisted_execution_requests: Option<String>,
    pub collection_assisted_execution_requests_latest: Option<String>,
    pub collection_assisted_execution_claims: Option<String>,
    pub collection_assisted_execution_claims_latest: Option<String>,
    pub collection_assisted_execution_attempts: Option<String>,
    pub collection_assisted_execution_attempts_latest: Option<String>,
}

impl AppConfig {
    pub fn from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let cfg: Self = toml::from_str(&content)?;
        Ok(cfg)
    }
}
