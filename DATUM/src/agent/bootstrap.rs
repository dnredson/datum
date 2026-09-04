use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::discovery::{BootstrapMode, SmartSentinelDiscovery};
use crate::agent::discovery_collectors::collect_discovery;
use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartSentinelBootstrapReport {
    pub discovery: SmartSentinelDiscovery,
    pub control_plane: ControlPlaneBootstrapContext,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPlaneBootstrapContext {
    pub dserver_url: String,
    pub project_id: String,
    pub node_known: bool,
    pub bootstrap_mode: BootstrapMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_capabilities: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentinel_registration: Option<Value>,
}

pub fn run_bootstrap(
    cfg: &Config,
    node_id: &str,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
) -> Result<SmartSentinelBootstrapReport> {
    if node_id.trim().is_empty() {
        bail!("SmartSentinel bootstrap node_id must not be empty");
    }
    if project_id.trim().is_empty() {
        bail!("SmartSentinel bootstrap project_id must not be empty");
    }
    if dserver_url.trim().is_empty() {
        bail!("SmartSentinel bootstrap dserver_url must not be empty");
    }
    if timeout_s == 0 {
        bail!("SmartSentinel bootstrap timeout_s must be greater than zero");
    }

    let base = dserver_url.trim().trim_end_matches('/');
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_s))
        .build()
        .context("build SmartSentinel bootstrap HTTP client")?;

    let node_url = format!("{base}/api/v1/nodes/{}/latest", node_id.trim());
    let node_response = client
        .get(&node_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver node context {node_url}"))?;
    let bootstrap_mode = classify_node_lookup_status(node_response.status())?;
    let node_known = bootstrap_mode == BootstrapMode::KnownNode;
    let latest_snapshot = if node_known {
        Some(
            node_response
                .json::<Value>()
                .context("decode known-node latest snapshot response")?,
        )
    } else {
        None
    };

    let capabilities_url = format!("{base}/api/v1/nodes/{}/capabilities", node_id.trim());
    let capabilities_response = client
        .get(&capabilities_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver node capabilities {capabilities_url}"))?;
    let expected_capabilities = optional_json_response(
        capabilities_response,
        "node capabilities",
        StatusCode::NOT_FOUND,
    )?;

    let registration_url = format!("{base}/api/v1/sentinels/{}", node_id.trim());
    let registration_response = client
        .get(&registration_url)
        .send()
        .with_context(|| format!("query dserver Sentinel registration {registration_url}"))?;
    let sentinel_registration = optional_json_response(
        registration_response,
        "Sentinel registration",
        StatusCode::NOT_FOUND,
    )?;

    let discovery = collect_discovery(node_id, hostname_override, bootstrap_mode, cfg)?;
    let control_plane = ControlPlaneBootstrapContext {
        dserver_url: base.to_string(),
        project_id: project_id.trim().to_string(),
        node_known,
        bootstrap_mode,
        latest_snapshot,
        expected_capabilities,
        sentinel_registration,
    };

    Ok(SmartSentinelBootstrapReport {
        discovery,
        control_plane,
        side_effects_performed: false,
    })
}

pub fn classify_node_lookup_status(status: StatusCode) -> Result<BootstrapMode> {
    match status {
        StatusCode::OK => Ok(BootstrapMode::KnownNode),
        StatusCode::NOT_FOUND => Ok(BootstrapMode::Discovery),
        other => bail!(
            "dserver node lookup returned unexpected status {}; refusing to guess bootstrap mode",
            other
        ),
    }
}

fn optional_json_response(
    response: reqwest::blocking::Response,
    label: &str,
    absent_status: StatusCode,
) -> Result<Option<Value>> {
    let status = response.status();
    if status == absent_status {
        return Ok(None);
    }
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("dserver {label} query failed with status {status}: {body}");
    }
    response
        .json::<Value>()
        .map(Some)
        .with_context(|| format!("decode dserver {label} response"))
}
