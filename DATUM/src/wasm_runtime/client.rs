use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{
    blocking::{Client, Response},
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Url,
};

use crate::wasm_runtime::report::{WasmRuntimeReport, WasmRuntimeReportList};

pub fn publish_wasm_runtime_report(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    report: &WasmRuntimeReport,
) -> Result<WasmRuntimeReport> {
    let endpoint = wasm_runtime_reports_publish_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(report)
        .send()
        .with_context(|| format!("publish WASM runtime report to {}", endpoint))?;

    parse_report_response(response, "publish WASM runtime report")
}

pub fn fetch_latest_wasm_runtime_report(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<WasmRuntimeReport> {
    let endpoint = wasm_runtime_report_latest_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest WASM runtime report from {}", endpoint))?;

    parse_report_response(response, "fetch latest WASM runtime report")
}

pub fn fetch_latest_wasm_runtime_reports_for_project(
    base_url: &str,
    project_id: &str,
) -> Result<WasmRuntimeReportList> {
    let endpoint = wasm_runtime_reports_latest_endpoint_url(base_url)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest WASM runtime reports from {}", endpoint))?;

    parse_report_list_response(response, "fetch latest WASM runtime reports")
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

fn parse_report_response(response: Response, label: &str) -> Result<WasmRuntimeReport> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_report_list_response(response: Response, label: &str) -> Result<WasmRuntimeReportList> {
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

pub fn wasm_runtime_reports_publish_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "nodes".to_string(),
        node_id.to_string(),
        "wasm-runtime-reports".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn wasm_runtime_report_latest_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "nodes".to_string(),
        node_id.to_string(),
        "wasm-runtime-reports".to_string(),
        "latest".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn wasm_runtime_reports_latest_endpoint_url(base_url: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend(["wasm-runtime-reports".to_string(), "latest".to_string()]);
    with_segments(base_url, &segments)
}

#[cfg(test)]
mod tests {
    use super::{
        wasm_runtime_report_latest_endpoint_url, wasm_runtime_reports_publish_endpoint_url,
    };
    use crate::wasm_runtime::report::WasmRuntimeReport;

    #[test]
    fn publish_url_construction_is_correct() {
        let url = wasm_runtime_reports_publish_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/nodes/fog-01/wasm-runtime-reports"
        );
    }

    #[test]
    fn latest_url_construction_is_correct() {
        let url = wasm_runtime_report_latest_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/nodes/fog-01/wasm-runtime-reports/latest"
        );
    }

    #[test]
    fn report_serialization_keeps_safety_and_access_flags_false() {
        let report = WasmRuntimeReport {
            project_id: None,
            report_id: Some("report-1".to_string()),
            node_id: None,
            reported_at_utc: None,
            source: Some("client".to_string()),
            module_name: "embedded:add".to_string(),
            function_name: "add".to_string(),
            execution_mode: "embedded_test".to_string(),
            status: "executed".to_string(),
            result_i32: Some(5),
            error_message: None,
            started_at_utc: None,
            finished_at_utc: None,
            duration_ms: Some(0),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            filesystem_access: false,
            network_access: false,
            host_command_access: false,
            docker_access: false,
            artifact_access: false,
            installation_task_access: false,
            assisted_execution_access: false,
            metadata: Some(serde_json::json!({
                "limits": {
                    "fuel_configured": 1000000,
                    "fuel_consumed": 3,
                    "timeout_ms": 1000,
                    "limit_exceeded": false
                }
            })),
        };

        let rendered =
            serde_json::to_string(&report).expect("WASM runtime report should serialize");

        assert!(rendered.contains("\"safe_to_auto_execute\":false"));
        assert!(rendered.contains("\"side_effects_performed\":false"));
        assert!(rendered.contains("\"filesystem_access\":false"));
        assert!(rendered.contains("\"network_access\":false"));
        assert!(rendered.contains("\"host_command_access\":false"));
        assert!(rendered.contains("\"docker_access\":false"));
        assert!(rendered.contains("\"artifact_access\":false"));
        assert!(rendered.contains("\"installation_task_access\":false"));
        assert!(rendered.contains("\"assisted_execution_access\":false"));
        assert!(rendered.contains("\"fuel_configured\":1000000"));
    }

    #[test]
    fn source_guard_confirms_no_installation_or_assisted_execution_references() {
        let source = include_str!("client.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section should exist");

        for forbidden in [
            "installation_tasks",
            "assisted_execution",
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "wasm runtime client must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
