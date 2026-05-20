use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Url,
};
use serde_json::json;

use crate::installation_tasks::model::{
    ApprovalRequest, AssistedExecutionAttempt, AssistedExecutionAttemptBody,
    AssistedExecutionAttemptList, AssistedExecutionClaim, AssistedExecutionClaimBody,
    AssistedExecutionClaimList, AssistedExecutionClaimStatusPatch, AssistedExecutionRequest,
    AssistedExecutionRequestBody, AssistedExecutionRequestList, InstallationTask,
    InstallationTaskApproval, InstallationTaskApprovalList, InstallationTaskList,
    InstallationTaskReadiness, InstallationTaskReadinessList,
};

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

pub fn submit_installation_task_approval(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    approved_by: Option<&str>,
    reason: Option<&str>,
    expires_at_utc: Option<&str>,
) -> Result<InstallationTaskApproval> {
    let endpoint = installation_task_approval_submit_endpoint_url(base_url, node_id, task_id)?;
    let client = build_client()?;
    let body = approval_request_body(approved_by, reason, expires_at_utc);
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .with_context(|| format!("submit installation task approval to {}", endpoint))?;

    parse_approval_response(response, "submit installation task approval")
}

pub fn fetch_latest_installation_task_approval(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
) -> Result<InstallationTaskApproval> {
    let endpoint = installation_task_approval_latest_endpoint_url(base_url, node_id, task_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest installation task approval from {}", endpoint))?;

    parse_approval_response(response, "fetch latest installation task approval")
}

pub fn fetch_installation_task_approvals_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<InstallationTaskApprovalList> {
    let endpoint = installation_task_approvals_list_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch installation task approvals from {}", endpoint))?;

    parse_approval_list_response(response, "fetch installation task approvals")
}

pub fn fetch_installation_task_readiness(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
) -> Result<InstallationTaskReadiness> {
    let endpoint = installation_task_readiness_endpoint_url(base_url, node_id, task_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch installation task readiness from {}", endpoint))?;

    parse_readiness_response(response, "fetch installation task readiness")
}

pub fn fetch_installation_task_readiness_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<InstallationTaskReadinessList> {
    let endpoint = installation_task_readiness_list_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch installation task readiness from {}", endpoint))?;

    parse_readiness_list_response(response, "fetch installation task readiness")
}

pub fn submit_assisted_execution_request(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    requested_by: Option<&str>,
    reason: Option<&str>,
) -> Result<AssistedExecutionRequest> {
    let endpoint = installation_task_assisted_execution_request_submit_endpoint_url(
        base_url, node_id, task_id,
    )?;
    let client = build_client()?;
    let body = assisted_execution_request_body(requested_by, reason);
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .with_context(|| format!("submit assisted execution request to {}", endpoint))?;

    parse_assisted_execution_request_response(response, "submit assisted execution request")
}

pub fn fetch_latest_assisted_execution_request(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
) -> Result<AssistedExecutionRequest> {
    let endpoint = installation_task_assisted_execution_request_latest_endpoint_url(
        base_url, node_id, task_id,
    )?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest assisted execution request from {}", endpoint))?;

    parse_assisted_execution_request_response(response, "fetch latest assisted execution request")
}

pub fn fetch_assisted_execution_requests_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<AssistedExecutionRequestList> {
    let endpoint = assisted_execution_requests_list_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch assisted execution requests from {}", endpoint))?;

    parse_assisted_execution_request_list_response(response, "fetch assisted execution requests")
}

pub fn submit_assisted_execution_claim(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
    claimed_by: Option<&str>,
    reason: Option<&str>,
    lease_expires_at_utc: Option<&str>,
) -> Result<AssistedExecutionClaim> {
    let endpoint = installation_task_assisted_execution_claim_submit_endpoint_url(
        base_url, node_id, task_id, request_id,
    )?;
    let client = build_client()?;
    let body = assisted_execution_claim_body(claimed_by, reason, lease_expires_at_utc);
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .with_context(|| format!("submit assisted execution claim to {}", endpoint))?;

    parse_assisted_execution_claim_response(response, "submit assisted execution claim")
}

pub fn fetch_latest_assisted_execution_claim(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
) -> Result<AssistedExecutionClaim> {
    let endpoint = installation_task_assisted_execution_claim_latest_endpoint_url(
        base_url, node_id, task_id, request_id,
    )?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest assisted execution claim from {}", endpoint))?;

    parse_assisted_execution_claim_response(response, "fetch latest assisted execution claim")
}

pub fn fetch_assisted_execution_claims_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<AssistedExecutionClaimList> {
    let endpoint = assisted_execution_claims_list_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch assisted execution claims from {}", endpoint))?;

    parse_assisted_execution_claim_list_response(response, "fetch assisted execution claims")
}

pub fn patch_assisted_execution_claim_status(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<AssistedExecutionClaim> {
    let endpoint = installation_task_assisted_execution_claim_status_endpoint_url(
        base_url, node_id, task_id, request_id,
    )?;
    let client = build_client()?;
    let body = assisted_execution_claim_status_patch_body(status, reason);
    let response = client
        .patch(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .with_context(|| format!("patch assisted execution claim status at {}", endpoint))?;

    parse_assisted_execution_claim_response(response, "patch assisted execution claim status")
}

pub fn submit_assisted_execution_attempt(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
    claim_id: &str,
    execution_mode: &str,
    attempted_by: Option<&str>,
    message: Option<&str>,
) -> Result<AssistedExecutionAttempt> {
    let endpoint = installation_task_assisted_execution_attempt_submit_endpoint_url(
        base_url, node_id, task_id, request_id, claim_id,
    )?;
    let client = build_client()?;
    let body = assisted_execution_attempt_body(execution_mode, attempted_by, message);
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .with_context(|| format!("submit assisted execution attempt to {}", endpoint))?;

    parse_assisted_execution_attempt_response(response, "submit assisted execution attempt")
}

pub fn fetch_latest_assisted_execution_attempt(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
    claim_id: &str,
) -> Result<AssistedExecutionAttempt> {
    let endpoint = installation_task_assisted_execution_attempt_latest_endpoint_url(
        base_url, node_id, task_id, request_id, claim_id,
    )?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest assisted execution attempt from {}", endpoint))?;

    parse_assisted_execution_attempt_response(response, "fetch latest assisted execution attempt")
}

pub fn fetch_assisted_execution_attempts_for_node(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<AssistedExecutionAttemptList> {
    let endpoint = assisted_execution_attempts_list_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch assisted execution attempts from {}", endpoint))?;

    parse_assisted_execution_attempt_list_response(response, "fetch assisted execution attempts")
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

fn parse_approval_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<InstallationTaskApproval> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_approval_list_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<InstallationTaskApprovalList> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_readiness_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<InstallationTaskReadiness> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_readiness_list_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<InstallationTaskReadinessList> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_assisted_execution_request_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<AssistedExecutionRequest> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_assisted_execution_request_list_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<AssistedExecutionRequestList> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_assisted_execution_claim_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<AssistedExecutionClaim> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_assisted_execution_claim_list_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<AssistedExecutionClaimList> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_assisted_execution_attempt_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<AssistedExecutionAttempt> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_assisted_execution_attempt_list_response(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<AssistedExecutionAttemptList> {
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

pub fn installation_task_approval_submit_endpoint_url(
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
        "approval".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_approval_latest_endpoint_url(
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
        "approval".to_string(),
        "latest".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_approvals_list_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-task-approvals".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_readiness_endpoint_url(
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
        "readiness".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_readiness_list_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-task-readiness".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_request_submit_endpoint_url(
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
        "assisted-execution-request".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_request_latest_endpoint_url(
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
        "assisted-execution-request".to_string(),
        "latest".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn assisted_execution_requests_list_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "assisted-execution-requests".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_claim_submit_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
        "assisted-execution-request".to_string(),
        request_id.to_string(),
        "claim".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_claim_latest_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
        "assisted-execution-request".to_string(),
        request_id.to_string(),
        "claim".to_string(),
        "latest".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_claim_status_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
        "assisted-execution-request".to_string(),
        request_id.to_string(),
        "claim".to_string(),
        "status".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn assisted_execution_claims_list_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "assisted-execution-claims".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_attempt_submit_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
    claim_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
        "assisted-execution-request".to_string(),
        request_id.to_string(),
        "claim".to_string(),
        claim_id.to_string(),
        "attempt".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn installation_task_assisted_execution_attempt_latest_endpoint_url(
    base_url: &str,
    node_id: &str,
    task_id: &str,
    request_id: &str,
    claim_id: &str,
) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "installation-tasks".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
        task_id.to_string(),
        "assisted-execution-request".to_string(),
        request_id.to_string(),
        "claim".to_string(),
        claim_id.to_string(),
        "attempt".to_string(),
        "latest".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn assisted_execution_attempts_list_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "assisted-execution-attempts".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
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

pub fn approval_request_body(
    approved_by: Option<&str>,
    reason: Option<&str>,
    expires_at_utc: Option<&str>,
) -> ApprovalRequest {
    ApprovalRequest {
        approved_by: approved_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        reason: reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        expires_at_utc: expires_at_utc
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

pub fn assisted_execution_request_body(
    requested_by: Option<&str>,
    reason: Option<&str>,
) -> AssistedExecutionRequestBody {
    AssistedExecutionRequestBody {
        requested_by: requested_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        reason: reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

pub fn assisted_execution_claim_body(
    claimed_by: Option<&str>,
    reason: Option<&str>,
    lease_expires_at_utc: Option<&str>,
) -> AssistedExecutionClaimBody {
    AssistedExecutionClaimBody {
        claimed_by: claimed_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        reason: reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        lease_expires_at_utc: lease_expires_at_utc
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

pub fn assisted_execution_claim_status_patch_body(
    status: &str,
    reason: Option<&str>,
) -> AssistedExecutionClaimStatusPatch {
    AssistedExecutionClaimStatusPatch {
        status: status.trim().to_string(),
        reason: reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

pub fn assisted_execution_attempt_body(
    execution_mode: &str,
    attempted_by: Option<&str>,
    message: Option<&str>,
) -> AssistedExecutionAttemptBody {
    AssistedExecutionAttemptBody {
        execution_mode: execution_mode.trim().to_string(),
        attempted_by: attempted_by
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        message: message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        approval_request_body, assisted_execution_attempt_body,
        assisted_execution_attempts_list_endpoint_url, assisted_execution_claim_body,
        assisted_execution_claim_status_patch_body, assisted_execution_claims_list_endpoint_url,
        assisted_execution_request_body, assisted_execution_requests_list_endpoint_url,
        installation_task_approval_latest_endpoint_url,
        installation_task_approval_submit_endpoint_url,
        installation_task_approvals_list_endpoint_url,
        installation_task_assisted_execution_attempt_latest_endpoint_url,
        installation_task_assisted_execution_attempt_submit_endpoint_url,
        installation_task_assisted_execution_claim_latest_endpoint_url,
        installation_task_assisted_execution_claim_status_endpoint_url,
        installation_task_assisted_execution_claim_submit_endpoint_url,
        installation_task_assisted_execution_request_latest_endpoint_url,
        installation_task_assisted_execution_request_submit_endpoint_url,
        installation_task_get_endpoint_url, installation_task_patch_endpoint_url,
        installation_task_readiness_endpoint_url, installation_task_readiness_list_endpoint_url,
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
    fn endpoint_construction_for_submit_approval_is_correct() {
        let url = installation_task_approval_submit_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/approval"
        );
    }

    #[test]
    fn endpoint_construction_for_latest_approval_is_correct() {
        let url = installation_task_approval_latest_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/approval/latest"
        );
    }

    #[test]
    fn endpoint_construction_for_list_approvals_is_correct() {
        let url = installation_task_approvals_list_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-task-approvals/for-node/fog-01"
        );
    }

    #[test]
    fn endpoint_construction_for_single_readiness_is_correct() {
        let url = installation_task_readiness_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/readiness"
        );
    }

    #[test]
    fn endpoint_construction_for_node_readiness_list_is_correct() {
        let url = installation_task_readiness_list_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-task-readiness/for-node/fog-01"
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
    fn approval_request_body_includes_optional_fields_when_provided() {
        let body = approval_request_body(
            Some("operator"),
            Some("approved for future assisted execution"),
            Some("2026-06-01T00:00:00Z"),
        );
        let value = serde_json::to_value(body).expect("body should serialize");

        assert_eq!(value["approved_by"].as_str(), Some("operator"));
        assert_eq!(
            value["reason"].as_str(),
            Some("approved for future assisted execution")
        );
        assert_eq!(
            value["expires_at_utc"].as_str(),
            Some("2026-06-01T00:00:00Z")
        );
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

    #[test]
    fn endpoint_construction_for_submit_assisted_execution_request_is_correct() {
        let url = installation_task_assisted_execution_request_submit_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/assisted-execution-request"
        );
    }

    #[test]
    fn endpoint_construction_for_latest_assisted_execution_request_is_correct() {
        let url = installation_task_assisted_execution_request_latest_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/assisted-execution-request/latest"
        );
    }

    #[test]
    fn endpoint_construction_for_list_assisted_execution_requests_is_correct() {
        let url = assisted_execution_requests_list_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/assisted-execution-requests/for-node/fog-01"
        );
    }

    #[test]
    fn assisted_execution_request_body_includes_optional_fields_when_provided() {
        let body = assisted_execution_request_body(
            Some("operator"),
            Some("request future assisted execution"),
        );
        let value = serde_json::to_value(body).expect("body should serialize");

        assert_eq!(value["requested_by"].as_str(), Some("operator"));
        assert_eq!(
            value["reason"].as_str(),
            Some("request future assisted execution")
        );
    }

    #[test]
    fn endpoint_construction_for_submit_assisted_execution_claim_is_correct() {
        let url = installation_task_assisted_execution_claim_submit_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "project:smart city/task",
            "request:with space",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/project:smart%20city%2Ftask/assisted-execution-request/request:with%20space/claim"
        );
    }

    #[test]
    fn endpoint_construction_for_latest_assisted_execution_claim_is_correct() {
        let url = installation_task_assisted_execution_claim_latest_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
            "request-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/assisted-execution-request/request-1/claim/latest"
        );
    }

    #[test]
    fn endpoint_construction_for_patch_assisted_execution_claim_status_is_correct() {
        let url = installation_task_assisted_execution_claim_status_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "project:smart city/task",
            "request:with space",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/project:smart%20city%2Ftask/assisted-execution-request/request:with%20space/claim/status"
        );
    }

    #[test]
    fn endpoint_construction_for_list_assisted_execution_claims_is_correct() {
        let url = assisted_execution_claims_list_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/assisted-execution-claims/for-node/fog-01"
        );
    }

    #[test]
    fn assisted_execution_claim_body_includes_optional_fields_when_provided() {
        let body = assisted_execution_claim_body(
            Some("operator"),
            Some("claim metadata for future assisted execution"),
            Some("2026-06-01T00:00:00Z"),
        );
        let value = serde_json::to_value(body).expect("body should serialize");

        assert_eq!(value["claimed_by"].as_str(), Some("operator"));
        assert_eq!(
            value["reason"].as_str(),
            Some("claim metadata for future assisted execution")
        );
        assert_eq!(
            value["lease_expires_at_utc"].as_str(),
            Some("2026-06-01T00:00:00Z")
        );
    }

    #[test]
    fn assisted_execution_claim_status_patch_body_includes_status_and_reason() {
        let body =
            assisted_execution_claim_status_patch_body("released", Some("operator released claim"));
        let value = serde_json::to_value(body).expect("body should serialize");

        assert_eq!(value["status"].as_str(), Some("released"));
        assert_eq!(value["reason"].as_str(), Some("operator released claim"));
    }

    #[test]
    fn endpoint_construction_for_submit_assisted_execution_attempt_is_correct() {
        let url = installation_task_assisted_execution_attempt_submit_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "project:smart city/task",
            "request:with space",
            "claim:with/slash",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/project:smart%20city%2Ftask/assisted-execution-request/request:with%20space/claim/claim:with%2Fslash/attempt"
        );
    }

    #[test]
    fn endpoint_construction_for_latest_assisted_execution_attempt_is_correct() {
        let url = installation_task_assisted_execution_attempt_latest_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
            "task-1",
            "request-1",
            "claim-1",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/installation-tasks/for-node/fog-01/task-1/assisted-execution-request/request-1/claim/claim-1/attempt/latest"
        );
    }

    #[test]
    fn endpoint_construction_for_list_assisted_execution_attempts_is_correct() {
        let url = assisted_execution_attempts_list_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/assisted-execution-attempts/for-node/fog-01"
        );
    }

    #[test]
    fn assisted_execution_attempt_body_includes_fields_when_provided() {
        let body = assisted_execution_attempt_body(
            "no_op",
            Some("operator"),
            Some("recording no-op attempt"),
        );
        let value = serde_json::to_value(body).expect("body should serialize");

        assert_eq!(value["execution_mode"].as_str(), Some("no_op"));
        assert_eq!(value["attempted_by"].as_str(), Some("operator"));
        assert_eq!(value["message"].as_str(), Some("recording no-op attempt"));
    }
}
