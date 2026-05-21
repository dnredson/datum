use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{
    blocking::{Client, Response},
    header::{HeaderMap, HeaderValue, CONTENT_TYPE},
    Url,
};

use crate::dgraph_snapshot::model::{LocalDGraphSnapshot, LocalDGraphSnapshotList};

pub fn publish_local_dgraph_snapshot(
    base_url: &str,
    project_id: &str,
    node_id: &str,
    snapshot: &LocalDGraphSnapshot,
) -> Result<LocalDGraphSnapshot> {
    let endpoint = local_dgraph_snapshot_publish_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .post(endpoint.clone())
        .headers(project_header(project_id)?)
        .header(CONTENT_TYPE, "application/json")
        .json(snapshot)
        .send()
        .with_context(|| format!("publish local d-graph snapshot to {}", endpoint))?;

    parse_snapshot_response(response, "publish local d-graph snapshot")
}

pub fn fetch_latest_local_dgraph_snapshot(
    base_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<LocalDGraphSnapshot> {
    let endpoint = local_dgraph_snapshot_latest_endpoint_url(base_url, node_id)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest local d-graph snapshot from {}", endpoint))?;

    parse_snapshot_response(response, "fetch latest local d-graph snapshot")
}

pub fn fetch_latest_local_dgraph_snapshots(
    base_url: &str,
    project_id: &str,
) -> Result<LocalDGraphSnapshotList> {
    let endpoint = local_dgraph_snapshots_latest_endpoint_url(base_url)?;
    let client = build_client()?;
    let response = client
        .get(endpoint.clone())
        .headers(project_header(project_id)?)
        .send()
        .with_context(|| format!("fetch latest local d-graph snapshots from {}", endpoint))?;

    parse_snapshot_list_response(response, "fetch latest local d-graph snapshots")
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

fn parse_snapshot_response(response: Response, label: &str) -> Result<LocalDGraphSnapshot> {
    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!("{label} failed: status={} body={}", status, body));
    }

    serde_json::from_str(&body).with_context(|| format!("parse {label} response JSON"))
}

fn parse_snapshot_list_response(
    response: Response,
    label: &str,
) -> Result<LocalDGraphSnapshotList> {
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

pub fn local_dgraph_snapshot_publish_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "nodes".to_string(),
        node_id.to_string(),
        "dgraph-snapshot".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn local_dgraph_snapshot_latest_endpoint_url(base_url: &str, node_id: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend([
        "nodes".to_string(),
        node_id.to_string(),
        "dgraph-snapshot".to_string(),
        "latest".to_string(),
    ]);
    with_segments(base_url, &segments)
}

pub fn local_dgraph_snapshots_latest_endpoint_url(base_url: &str) -> Result<Url> {
    let mut segments = base_api_segments(base_url)?;
    segments.extend(["dgraph-snapshots".to_string(), "latest".to_string()]);
    with_segments(base_url, &segments)
}

#[cfg(test)]
mod tests {
    use super::{
        local_dgraph_snapshot_latest_endpoint_url, local_dgraph_snapshot_publish_endpoint_url,
        local_dgraph_snapshots_latest_endpoint_url,
    };
    use crate::dgraph_snapshot::model::{
        LocalCapabilitySnapshot, LocalDGraphSnapshot, LocalFlowSnapshot, LocalHealthSnapshot,
        LocalModuleSnapshot, LocalRuntimeSnapshot,
    };

    #[test]
    fn publish_url_construction_is_correct() {
        let url = local_dgraph_snapshot_publish_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/nodes/fog-01/dgraph-snapshot"
        );
    }

    #[test]
    fn latest_url_construction_is_correct() {
        let url = local_dgraph_snapshot_latest_endpoint_url(
            "http://localhost:8080/api/v1/snapshots",
            "fog-01",
        )
        .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/nodes/fog-01/dgraph-snapshot/latest"
        );
    }

    #[test]
    fn project_latest_url_construction_is_correct() {
        let url =
            local_dgraph_snapshots_latest_endpoint_url("http://localhost:8080/api/v1/snapshots")
                .expect("endpoint should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/dgraph-snapshots/latest"
        );
    }

    #[test]
    fn publish_request_serialization_keeps_safety_flags_false() {
        let snapshot = LocalDGraphSnapshot {
            node_id: "fog-01".to_string(),
            project_id: Some("smart-city-campinas".to_string()),
            snapshot_id: None,
            reported_at_utc: None,
            source: None,
            generated_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: "runtime-container-local".to_string(),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "local_config".to_string(),
                metadata: None,
            }],
            modules: vec![LocalModuleSnapshot {
                module_id: "chirpstack-primary".to_string(),
                module_kind: Some("service".to_string()),
                desired_state: Some("running".to_string()),
                observed_state: Some("unknown".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: Some("nginx:stable".to_string()),
                local_mapping_key: Some("chirpstack-primary".to_string()),
                local_container_name: Some("chirpstack".to_string()),
                source: "service_registry".to_string(),
                metadata: None,
            }],
            flows: vec![LocalFlowSnapshot {
                flow_id: "configured-service-flow".to_string(),
                flow_kind: Some("service_dependency".to_string()),
                modules: vec!["chirpstack-primary".to_string()],
                edges: vec![],
                status: Some("configured".to_string()),
                source: "local_config".to_string(),
                metadata: None,
            }],
            health: vec![LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: "chirpstack-primary".to_string(),
                status: "unknown".to_string(),
                reason: None,
                source: "local_config".to_string(),
                metadata: None,
            }],
            capabilities: vec![LocalCapabilitySnapshot {
                capability_id: "capability-container-runtime-configured".to_string(),
                capability_kind: "container_runtime_configured".to_string(),
                supported: true,
                source: "local_config".to_string(),
                metadata: None,
            }],
            metadata: Some(serde_json::json!({ "reporting_only": true })),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        };

        let rendered = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        assert!(rendered.contains("\"safe_to_auto_execute\":false"));
        assert!(rendered.contains("\"side_effects_performed\":false"));
    }

    #[test]
    fn source_guard_confirms_no_docker_shell_config_write_or_wasm_execution() {
        let source = include_str!("client.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section should exist");

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
            "run_embedded_wasm",
            "wasm_runtime::runner",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "dgraph snapshot client must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
