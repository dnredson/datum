use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    api::operational_analysis_context::acquire_passive_analysis_context_for_node,
    core::operational_analysis_context::OperationalAnalysisContext,
    AppState,
};

type ApiError = (StatusCode, Json<Value>);

pub const PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Deserialize)]
pub struct PassiveObservationNotification {
    pub schema_version: String,
    pub node_id: String,
    pub cycle_id: String,
    pub observed_at_utc: String,
}

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

pub async fn post_passive_observation_notification(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(notification): Json<PassiveObservationNotification>,
) -> Result<Json<PassiveObservationTriggerResult>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    validate_notification(&notification).map_err(bad_request)?;

    Ok(Json(
        process_passive_observation_trigger(
            &state,
            &project_id,
            &notification.node_id,
            &notification.cycle_id,
            &notification.observed_at_utc,
            "firebase_notification",
        )
        .await,
    ))
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
                schema_version: PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION.to_string(),
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
            schema_version: PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION.to_string(),
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

fn validate_notification(notification: &PassiveObservationNotification) -> Result<(), String> {
    if notification.schema_version.trim() != PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION {
        return Err(format!(
            "unsupported passive observation notification schema_version '{}'; expected '{}'",
            notification.schema_version, PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION
        ));
    }
    if notification.node_id.trim().is_empty() {
        return Err("passive observation notification node_id must not be empty".to_string());
    }
    if notification.cycle_id.trim().is_empty() {
        return Err("passive observation notification cycle_id must not be empty".to_string());
    }
    if notification.observed_at_utc.trim().is_empty() {
        return Err(
            "passive observation notification observed_at_utc must not be empty".to_string(),
        );
    }
    DateTime::parse_from_rfc3339(notification.observed_at_utc.trim()).map_err(|error| {
        format!(
            "passive observation notification observed_at_utc must be RFC3339: {error}"
        )
    })?;
    Ok(())
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

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| bad_request("missing or invalid X-Datum-Project header".to_string()))
}

fn bad_request(message: String) -> ApiError {
    (StatusCode::BAD_REQUEST, Json(json!({"error": message})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_validation_accepts_rfc3339_contract() {
        let notification = PassiveObservationNotification {
            schema_version: PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION.to_string(),
            node_id: "fog-01".to_string(),
            cycle_id: "cycle-1".to_string(),
            observed_at_utc: "2026-09-06T20:30:00Z".to_string(),
        };
        assert!(validate_notification(&notification).is_ok());
    }

    #[test]
    fn notification_validation_rejects_invalid_timestamp() {
        let notification = PassiveObservationNotification {
            schema_version: PASSIVE_OBSERVATION_NOTIFICATION_SCHEMA_VERSION.to_string(),
            node_id: "fog-01".to_string(),
            cycle_id: "cycle-1".to_string(),
            observed_at_utc: "not-a-time".to_string(),
        };
        assert!(validate_notification(&notification).is_err());
    }

    #[test]
    fn equivalent_rfc3339_timestamps_match() {
        assert!(timestamps_match(
            "2026-09-06T20:30:00Z",
            "2026-09-06T17:30:00-03:00"
        ));
    }
}
