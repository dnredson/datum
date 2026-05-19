use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};

use crate::deployment_planning::model::DeploymentPlanningResponse;

pub fn fetch_deployment_planning_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<DeploymentPlanningResponse> {
    let endpoint = deployment_planning_endpoint_url(base_url, node_id)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build blocking reqwest client")?;

    let response = client
        .get(endpoint.clone())
        .header("X-Datum-Project", project_id)
        .send()
        .with_context(|| format!("fetch deployment planning from {}", endpoint))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!(
            "deployment planning request failed: status={} body={}",
            status,
            body
        ));
    }

    serde_json::from_str(&body).context("parse deployment planning response JSON")
}

pub fn deployment_planning_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse base URL: {base_url}"))?;

    let mut segments = url
        .path_segments()
        .ok_or_else(|| anyhow!("base URL must be an absolute HTTP URL"))?
        .map(str::to_string)
        .collect::<Vec<_>>();

    if segments.last().map(|s| s.as_str()) == Some("snapshots") {
        segments.pop();
    }

    segments.extend([
        "deployment-planning".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);

    url.path_segments_mut()
        .map_err(|_| anyhow!("base URL cannot be a base URL"))?
        .clear()
        .extend(segments.iter().map(|s| s.as_str()));

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::deployment_planning_endpoint_url;

    #[test]
    fn endpoint_construction_is_correct() {
        let url =
            deployment_planning_endpoint_url("http://localhost:8080/api/v1/snapshots", "fog-01")
                .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/deployment-planning/for-node/fog-01"
        );
    }
}
