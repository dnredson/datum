use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};
use serde_json::Value;

use crate::{capabilities::model::NodeCapabilities, config::HttpConfig};

pub fn publish_node_capabilities(
    outputs_http: &HttpConfig,
    project_id: &str,
    node_id: &str,
    capabilities: &NodeCapabilities,
) -> Result<Value> {
    let endpoint = capabilities_endpoint_url(outputs_http, node_id)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(outputs_http.timeout_s.unwrap_or(10)))
        .build()
        .context("build blocking reqwest client")?;

    let response = client
        .post(endpoint.clone())
        .header("Content-Type", "application/json")
        .header("X-Datum-Project", project_id)
        .json(capabilities)
        .send()
        .with_context(|| format!("publish capabilities to {}", endpoint))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!(
            "capability publish failed: status={} body={}",
            status,
            body
        ));
    }

    serde_json::from_str(&body).context("parse capability publish response JSON")
}

pub fn capabilities_endpoint_url(outputs_http: &HttpConfig, node_id: &str) -> Result<Url> {
    let mut url = Url::parse(&outputs_http.url)
        .with_context(|| format!("parse outputs.http.url: {}", outputs_http.url))?;

    let mut segments = url
        .path_segments()
        .ok_or_else(|| anyhow!("outputs.http.url must be an absolute HTTP URL"))?
        .map(str::to_string)
        .collect::<Vec<_>>();

    if segments.last().map(|s| s.as_str()) == Some("snapshots") {
        segments.pop();
    }

    segments.extend([
        "nodes".to_string(),
        node_id.to_string(),
        "capabilities".to_string(),
    ]);

    url.path_segments_mut()
        .map_err(|_| anyhow!("outputs.http.url cannot be a base URL"))?
        .clear()
        .extend(segments.iter().map(|s| s.as_str()));

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::capabilities_endpoint_url;
    use crate::config::HttpConfig;

    #[test]
    fn endpoint_construction_is_correct() {
        let cfg = HttpConfig {
            enabled: true,
            url: "http://localhost:8080/api/v1/snapshots".to_string(),
            project_id: Some("smart-city-campinas".to_string()),
            timeout_s: Some(10),
        };

        let url = capabilities_endpoint_url(&cfg, "fog-01").expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/nodes/fog-01/capabilities"
        );
    }
}
