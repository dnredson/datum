use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const OPERATIONAL_EVIDENCE_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalEvidenceEvent {
    pub schema_version: String,
    pub event_id: String,
    pub occurred_at_utc: String,
    pub project_id: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dgraph_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dgraph_revision: Option<u64>,
    pub sequence: u64,
    pub event_kind: String,
    pub side_effects_performed: bool,
    pub payload: Value,
}

impl OperationalEvidenceEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: &str,
        resource_id: &str,
        node_id: Option<&str>,
        dgraph_id: Option<&str>,
        dgraph_revision: Option<u64>,
        sequence: u64,
        event_kind: &str,
        side_effects_performed: bool,
        payload: Value,
    ) -> Result<Self> {
        if sequence == 0 {
            bail!("operational evidence sequence must be greater than zero");
        }
        if dgraph_revision == Some(0) {
            bail!("operational evidence dgraph_revision must be greater than zero");
        }
        if dgraph_revision.is_some() && dgraph_id.is_none() {
            bail!("operational evidence dgraph_revision requires dgraph_id");
        }
        if !payload.is_object() {
            bail!("operational evidence payload must be a JSON object");
        }
        let event = Self {
            schema_version: OPERATIONAL_EVIDENCE_SCHEMA_VERSION.to_string(),
            event_id: format!("opev:{}", uuid::Uuid::new_v4()),
            occurred_at_utc: Utc::now().to_rfc3339(),
            project_id: non_empty("project_id", project_id)?.to_string(),
            resource_id: non_empty("resource_id", resource_id)?.to_string(),
            node_id: optional_non_empty(node_id),
            dgraph_id: optional_non_empty(dgraph_id),
            dgraph_revision,
            sequence,
            event_kind: non_empty("event_kind", event_kind)?.to_string(),
            side_effects_performed,
            payload,
        };
        Ok(event)
    }
}

pub fn append_operational_evidence(path: &Path, event: &OperationalEvidenceEvent) -> Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create operational evidence directory {}", parent.display()))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("open operational evidence history {}", path.display()))?;
    let json = serde_json::to_string(event).context("serialize operational evidence event")?;
    writeln!(file, "{json}")
        .with_context(|| format!("append operational evidence history {}", path.display()))?;
    file.flush()
        .with_context(|| format!("flush operational evidence history {}", path.display()))
}

pub fn publish_operational_evidence(
    client: &Client,
    dserver_url: &str,
    project_id: &str,
    event: &OperationalEvidenceEvent,
) -> Result<()> {
    let url = format!(
        "{}/api/v1/operational-evidence",
        dserver_url.trim().trim_end_matches('/')
    );
    let response = client
        .post(&url)
        .header("X-Datum-Project", project_id.trim())
        .json(event)
        .send()
        .with_context(|| format!("publish operational evidence {url}"))?;
    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let body = response.text().unwrap_or_default();
    bail!("dserver operational evidence publish failed with status {status}: {body}")
}

fn non_empty<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("operational evidence {label} must not be empty");
    }
    Ok(value)
}

fn optional_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn evidence_event_captures_action_context() {
        let event = OperationalEvidenceEvent::new(
            "project-a",
            "resource:os:abc",
            Some("fog-01"),
            Some("dgraph:v1"),
            Some(2),
            1,
            "reconciliation",
            true,
            json!({"execution_status":"deployed"}),
        )
        .unwrap();
        assert_eq!(event.node_id.as_deref(), Some("fog-01"));
        assert_eq!(event.dgraph_revision, Some(2));
        assert!(event.side_effects_performed);
        assert!(event.event_id.starts_with("opev:"));
    }
}
