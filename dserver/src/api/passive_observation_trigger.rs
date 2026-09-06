use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::{
    api::operational_analysis_context::acquire_passive_analysis_context_for_node,
    core::operational_analysis_context::OperationalAnalysisContext,
    AppState,
};

pub const PASSIVE_OBSERVATION_TRIGGER_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize)]
pub struct PassiveObservationTriggerResult {
    pub schema_version: String,
    pub trigger_source: String,
    pub accepted: bool,
    pub project_id: String,
    pub node_id: String,
    pub notification_cycle_id: String,
    pub notification_observed_at_utc: String,
    pub processed_at_utc: String,
    pub analysis_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_cycle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_observed_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification_matches_latest: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment: Option<OperationalAnalysisContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis_error: Option<String>,
    pub side_effects_performed: bool,
}

pub async fn process_passive_observation_trigger(
    state: &AppState,
    project_id: &str,
    node_id: &str,
    cycle_id: &str,
    observed_at_utc: &str,
    trigger_source: &str,
) -> PassiveObservationTriggerResult {
    let processed_at_utc = Utc::now().to_rfc3339();

    match acquire_passive_analysis_context_for_node(state, project_id, node_id).await {
        Ok(assessment) => {
            let latest_node = assessment.nodes.first();
            let latest_observed_at_utc = latest_node
                .and_then(|node| node.telemetry_observed_at_utc.clone());
            let latest_cycle_id = latest_node
                .and_then(|node| node.latest_telemetry.as_ref())
                .and_then(firestore_cycle_id);
            let notification_matches_latest = latest_observed_at_utc
                .as_deref()
                .map(|latest| timestamps_match(observed_at_utc, latest));

            PassiveObservationTriggerResult {
                schema_version: PASSIVE_OBSERVATION_TRIGGER_SCHEMA_VERSION.to_string(),
                trigger_source: trigger_source.to_string(),
                accepted: true,
                project_id: project_id.to_string(),
                node_id: node_id.to_string(),
                notification_cycle_id: cycle_id.to_string(),
                notification_observed_at_utc: observed_at_utc.to_string(),
                processed_at_utc,
                analysis_status: "assessed".to_string(),
                latest_cycle_id,
                latest_observed_at_utc,
                notification_matches_latest,
                assessment: Some(assessment),
                analysis_error: None,
                side_effects_performed: false,
            }
        }
        Err(error) => PassiveObservationTriggerResult {
            schema_version: PASSIVE_OBSERVATION_TRIGGER_SCHEMA_VERSION.to_string(),
            trigger_source: trigger_source.to_string(),
            accepted: true,
            project_id: project_id.to_string(),
            node_id: node_id.to_string(),
            notification_cycle_id: cycle_id.to_string(),
            notification_observed_at_utc: observed_at_utc.to_string(),
            processed_at_utc,
            analysis_status: "not_assessed".to_string(),
            latest_cycle_id: None,
            latest_observed_at_utc: None,
            notification_matches_latest: None,
            assessment: None,
            analysis_error: Some(error),
            side_effects_performed: false,
        },
    }
}

fn firestore_cycle_id(document: &Value) -> Option<String> {
    document
        .get("fields")
        .and_then(|fields| fields.get("cycle_id"))
        .and_then(|value| value.get("stringValue"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn timestamps_match(left: &str, right: &str) -> bool {
    match (
        DateTime::parse_from_rfc3339(left.trim()),
        DateTime::parse_from_rfc3339(right.trim()),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.trim() == right.trim(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equivalent_rfc3339_timestamps_match() {
        assert!(timestamps_match(
            "2026-09-06T20:30:00Z",
            "2026-09-06T17:30:00-03:00"
        ));
    }

    #[test]
    fn different_timestamps_do_not_match() {
        assert!(!timestamps_match(
            "2026-09-06T20:30:00Z",
            "2026-09-06T20:31:00Z"
        ));
    }
}
