use chrono::DateTime;
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

pub fn validate_operational_evidence_event(event: &OperationalEvidenceEvent) -> Result<(), String> {
    required("schema_version", &event.schema_version)?;
    if event.schema_version != OPERATIONAL_EVIDENCE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported operational evidence schema_version '{}'; expected '{}'",
            event.schema_version, OPERATIONAL_EVIDENCE_SCHEMA_VERSION
        ));
    }
    required("event_id", &event.event_id)?;
    required("occurred_at_utc", &event.occurred_at_utc)?;
    DateTime::parse_from_rfc3339(event.occurred_at_utc.trim())
        .map_err(|error| format!("operational evidence occurred_at_utc must be RFC3339: {error}"))?;
    required("project_id", &event.project_id)?;
    required("resource_id", &event.resource_id)?;
    required("event_kind", &event.event_kind)?;
    if event.sequence == 0 {
        return Err("operational evidence sequence must be greater than zero".to_string());
    }
    if let Some(node_id) = event.node_id.as_deref() {
        required("node_id", node_id)?;
    }
    if let Some(dgraph_id) = event.dgraph_id.as_deref() {
        required("dgraph_id", dgraph_id)?;
    }
    if event.dgraph_revision == Some(0) {
        return Err("operational evidence dgraph_revision must be greater than zero".to_string());
    }
    if event.dgraph_revision.is_some() && event.dgraph_id.is_none() {
        return Err("operational evidence dgraph_revision requires dgraph_id".to_string());
    }
    if !event.payload.is_object() {
        return Err("operational evidence payload must be a JSON object".to_string());
    }
    Ok(())
}

fn required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("operational evidence {label} must not be empty"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event() -> OperationalEvidenceEvent {
        OperationalEvidenceEvent {
            schema_version: OPERATIONAL_EVIDENCE_SCHEMA_VERSION.to_string(),
            event_id: "opev:test".to_string(),
            occurred_at_utc: "2026-09-04T20:30:00Z".to_string(),
            project_id: "project-a".to_string(),
            resource_id: "resource:os:abc".to_string(),
            node_id: Some("fog-01".to_string()),
            dgraph_id: Some("dgraph:test".to_string()),
            dgraph_revision: Some(2),
            sequence: 1,
            event_kind: "reconciliation".to_string(),
            side_effects_performed: true,
            payload: json!({"execution_status":"deployed"}),
        }
    }

    #[test]
    fn valid_operational_evidence_is_accepted() {
        assert!(validate_operational_evidence_event(&event()).is_ok());
    }

    #[test]
    fn zero_sequence_is_rejected() {
        let mut value = event();
        value.sequence = 0;
        assert!(validate_operational_evidence_event(&value).is_err());
    }

    #[test]
    fn revision_requires_graph_identity() {
        let mut value = event();
        value.dgraph_id = None;
        assert!(validate_operational_evidence_event(&value).is_err());
    }
}
