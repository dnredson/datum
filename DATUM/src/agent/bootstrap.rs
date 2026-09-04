use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::resource_discovery::{
    collect_unbound_resource_discovery, derive_resource_lookup_id,
    SmartSentinelResourceDiscovery,
};
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapState {
    KnownBoundNode,
    KnownUnboundResource,
    UnknownResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartSentinelBootstrapReport {
    pub bootstrap_state: BootstrapState,
    pub resource_discovery: SmartSentinelResourceDiscovery,
    pub control_plane: ControlPlaneBootstrapContext,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPlaneBootstrapContext {
    pub dserver_url: String,
    pub project_id: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_node_id: Option<String>,
    pub node_known: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_record: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_capabilities: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentinel_registration: Option<Value>,
}

pub fn run_bootstrap(
    cfg: &Config,
    requested_node_id: Option<&str>,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
) -> Result<SmartSentinelBootstrapReport> {
    if project_id.trim().is_empty() {
        bail!("SmartSentinel bootstrap project_id must not be empty");
    }
    if dserver_url.trim().is_empty() {
        bail!("SmartSentinel bootstrap dserver_url must not be empty");
    }
    if timeout_s == 0 {
        bail!("SmartSentinel bootstrap timeout_s must be greater than zero");
    }

    let requested_node_id = requested_node_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let base = dserver_url.trim().trim_end_matches('/');
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_s))
        .build()
        .context("build SmartSentinel bootstrap HTTP client")?;

    let resource_discovery = collect_unbound_resource_discovery(cfg, hostname_override)?;
    let resource_id = derive_resource_lookup_id(&resource_discovery);
    let resource_url = format!("{base}/api/v1/resources/{resource_id}");
    let resource_response = client
        .get(&resource_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver resource context {resource_url}"))?;
    let resource_record = optional_json_response(
        resource_response,
        "resource registry",
        StatusCode::NOT_FOUND,
    )?;

    let registry_assigned_node_id = resource_record
        .as_ref()
        .and_then(|value| value.get("assigned_node_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    if let (Some(requested), Some(assigned)) = (
        requested_node_id.as_deref(),
        registry_assigned_node_id.as_deref(),
    ) {
        if requested != assigned {
            bail!(
                "resource registry binds local resource '{}' to node '{}' but bootstrap requested node '{}'; refusing conflicting role",
                resource_id,
                assigned,
                requested
            );
        }
    }

    let node_candidate = registry_assigned_node_id
        .clone()
        .or_else(|| requested_node_id.clone());
    let (node_known, latest_snapshot) = match node_candidate.as_deref() {
        Some(node_id) => query_node_context(&client, base, project_id, node_id)?,
        None => (false, None),
    };

    let bootstrap_state = classify_bootstrap_state(
        node_known,
        resource_record.is_some(),
        registry_assigned_node_id.is_some(),
    );

    let resolved_node_id = if node_known {
        node_candidate.clone()
    } else {
        registry_assigned_node_id.clone()
    };

    let (expected_capabilities, sentinel_registration) = match resolved_node_id.as_deref() {
        Some(node_id) => (
            query_optional_capabilities(&client, base, project_id, node_id)?,
            query_optional_registration(&client, base, node_id)?,
        ),
        None => (None, None),
    };

    let control_plane = ControlPlaneBootstrapContext {
        dserver_url: base.to_string(),
        project_id: project_id.trim().to_string(),
        resource_id,
        requested_node_id,
        resolved_node_id,
        node_known,
        resource_record,
        latest_snapshot,
        expected_capabilities,
        sentinel_registration,
    };

    Ok(SmartSentinelBootstrapReport {
        bootstrap_state,
        resource_discovery,
        control_plane,
        side_effects_performed: false,
    })
}

pub fn classify_bootstrap_state(
    node_known: bool,
    resource_known: bool,
    resource_has_assigned_node: bool,
) -> BootstrapState {
    if node_known || resource_has_assigned_node {
        BootstrapState::KnownBoundNode
    } else if resource_known {
        BootstrapState::KnownUnboundResource
    } else {
        BootstrapState::UnknownResource
    }
}

fn query_node_context(
    client: &Client,
    base: &str,
    project_id: &str,
    node_id: &str,
) -> Result<(bool, Option<Value>)> {
    let node_url = format!("{base}/api/v1/nodes/{}/latest", node_id.trim());
    let response = client
        .get(&node_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver node context {node_url}"))?;
    match response.status() {
        StatusCode::OK => Ok((
            true,
            Some(
                response
                    .json::<Value>()
                    .context("decode known-node latest snapshot response")?,
            ),
        )),
        StatusCode::NOT_FOUND => Ok((false, None)),
        other => {
            let body = response.text().unwrap_or_default();
            bail!(
                "dserver node lookup returned unexpected status {other}; refusing to guess bootstrap state: {body}"
            )
        }
    }
}

fn query_optional_capabilities(
    client: &Client,
    base: &str,
    project_id: &str,
    node_id: &str,
) -> Result<Option<Value>> {
    let url = format!("{base}/api/v1/nodes/{}/capabilities", node_id.trim());
    let response = client
        .get(&url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver node capabilities {url}"))?;
    optional_json_response(response, "node capabilities", StatusCode::NOT_FOUND)
}

fn query_optional_registration(
    client: &Client,
    base: &str,
    node_id: &str,
) -> Result<Option<Value>> {
    let url = format!("{base}/api/v1/sentinels/{}", node_id.trim());
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("query dserver Sentinel registration {url}"))?;
    optional_json_response(response, "Sentinel registration", StatusCode::NOT_FOUND)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_classification_distinguishes_three_resource_states() {
        assert_eq!(
            classify_bootstrap_state(true, true, true),
            BootstrapState::KnownBoundNode
        );
        assert_eq!(
            classify_bootstrap_state(false, true, true),
            BootstrapState::KnownBoundNode
        );
        assert_eq!(
            classify_bootstrap_state(false, true, false),
            BootstrapState::KnownUnboundResource
        );
        assert_eq!(
            classify_bootstrap_state(false, false, false),
            BootstrapState::UnknownResource
        );
    }
}
