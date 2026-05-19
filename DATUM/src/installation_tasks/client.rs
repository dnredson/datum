use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Url,
};
use serde_json::json;

use crate::installation_tasks::model::{InstallationTask, InstallationTaskList};

pub fn materialize_installation_tasks_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<InstallationTaskList> {
    let endpoint = installation_tasks_materialize_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .with_context(|| format!("materialize installation tasks from {}", endpoint))?;

    parse_task_list_response(response, "materialize installation tasks")
}

pub fn fetch_installation_tasks_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<InstallationTaskList> {
    let endpoint = installation_tasks_list_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch installation tasks from {}", endpoint))?;

    parse_task_list_response(response, "fetch installation tasks")
}

pub fn fetch_installation_task(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
) -> Result<InstallationTask> {
    let endpoint = installation_task_get_endpoint_url(base_url, node_id, task_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch installation task from {}", endpoint))?;

    parse_task_response(response, "fetch installation task")
}

pub fn patch_installation_task_status(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<InstallationTask> {
    let endpoint = installation_task_patch_endpoint_url(base_url, node_id, task_id)?;
    let client = build_client()?;
    let body = patch_status_body(status, reason);
    let response = client
        .patch(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .with_context(|| format!("patch installation task status at {}", endpoint))?;

    parse_task_response(response, "patch installation task status")
}

fn build_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build blocking reqwest client")
}

fn project_header(project_id: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Datum-Project",
        HeaderValue::from_str(project_id).context("build X-Datum-Project header value")?,
    );
    Ok(headers)
}

fn parse_task_list_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<InstallationTaskList> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_task_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<InstallationTask> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn base_api_segments(base_url: &str) -> Result<Vec<String>> {
    let url = Url::parse(base_url).with_context(|| format!("parse base URL: {base_url}"))?;
    let mut segments = url
        .path_segments()
        .ok_or_else(|| anyhow!("base URL must be an absolute HTTP URL"))?
        .map(str::to_string)
        .collect::<Vec<_>>();

    if segments.last().map(|s| s.as_str()) == Some("snapshots") {
        segments.pop();
    }

    Ok(segments)
}

fn with_segments(base_url: &str, segments: &[String]) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse base URL: {base_url}"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("base URL cannot be a base URL"))?
        .clear()
        .extend(segments.iter().map(|s| s.as_str()));
    Ok(url)
}

pub fn installation_tasks_materialize_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "materialize".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_tasks_list_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_get_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_patch_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
        "status".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn patch_status_body(status: &str, reason: Option<&str>) -> serde_json::Value {
    match reason.map(str::trim).filter(|value| !value.is_empty()) {
        Some(reason) => json!({
            "status": status,
            "reason": reason
        }),
        None => json!({
            "status": status
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        installation_task_get_endpoint_url, installation_task_patch_endpoint_url,
        installation_tasks_list_endpoint_url, installation_tasks_materialize_endpoint_url,
        patch_status_body, project_header,
    };

    #[test]
    fn endpoint_construction_for_materialize_is_correct() {
        let url = installation_tasks_materialize_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/materialize/for-node/fog-01"
        );
    }

    #[test]
    fn endpoint_construction_for_list_is_correct() {
        let url = installation_tasks_list_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01"
        );
    }

    #[test]
    fn endpoint_construction_for_get_is_correct() {
        let url = installation_task_get_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "project:smart-city-campinas:node:fog-01:installation:installation:planning:1:prepare_local_mapping",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/project:smart-city-campinas:node:fog-01:installation:installation:planning:1:prepare_local_mapping"
        );
    }

    #[test]
    fn endpoint_construction_for_patch_is_correct() {
        let url = installation_task_patch_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/status"
        );
    }

    #[test]
    fn patch_body_includes_status_and_optional_reason() {
        let with_reason = patch_status_body("acknowledged", Some("operator reviewed this task"));
        let without_reason = patch_status_body("blocked", None);

        assert_eq!(with_reason["status"].as_str(), Some("acknowledged"));
        assert_eq!(
            with_reason["reason"].as_str(),
            Some("operator reviewed this task")
        );
        assert_eq!(without_reason["status"].as_str(), Some("blocked"));
        assert!(without_reason.get("reason").is_none());
    }

    #[test]
    fn x_datum_project_header_is_built() {
        let headers = project_header("smart-city-campinas").expect("header should build");

        assert_eq!(
            headers
                .get("X-Datum-Project")
                .and_then(|value| value.to_str().ok()),
            Some("smart-city-campinas")
        );
    }
}
