use std::{
    collections::{BTreeMap, VecDeque},
    sync::{OnceLock, RwLock},
};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{json, Value};

pub const SENTINEL_TELEMETRY_SCHEMA_VERSION: &str = "0.1.0";
pub const DEFAULT_TELEMETRY_HISTORY_CAPACITY: usize = 256;
pub const DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS: i64 = 15;
pub const DEFAULT_TELEMETRY_FUTURE_SKEW_SECONDS: i64 = 60;
pub const MAX_TELEMETRY_PAYLOAD_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TelemetryObservationView {
    pub schema_version: String,
    pub project_id: String,
    pub node_id: String,
    pub sentinel_instance_id: String,
    pub sequence: u64,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub telemetry_age_seconds: u64,
    pub telemetry_freshness: String,
    pub payload: Value,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryIngestDisposition {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone)]
struct TelemetryObservation {
    project_id: String,
    node_id: String,
    sentinel_instance_id: String,
    sequence: u64,
    observed_at_utc: DateTime<Utc>,
    received_at_utc: DateTime<Utc>,
    payload: Value,
}

impl TelemetryObservation {
    fn view(&self, now: DateTime<Utc>) -> TelemetryObservationView {
        let age_seconds = now
            .signed_duration_since(self.observed_at_utc)
            .num_seconds()
            .max(0) as u64;
        let freshness = if age_seconds <= DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS as u64 {
            "fresh"
        } else {
            "stale"
        };

        TelemetryObservationView {
            schema_version: SENTINEL_TELEMETRY_SCHEMA_VERSION.to_string(),
            project_id: self.project_id.clone(),
            node_id: self.node_id.clone(),
            sentinel_instance_id: self.sentinel_instance_id.clone(),
            sequence: self.sequence,
            observed_at_utc: self.observed_at_utc.to_rfc3339(),
            received_at_utc: self.received_at_utc.to_rfc3339(),
            telemetry_age_seconds: age_seconds,
            telemetry_freshness: freshness.to_string(),
            payload: self.payload.clone(),
            side_effects_performed: false,
        }
    }
}

type ProjectHistory = BTreeMap<String, VecDeque<TelemetryObservation>>;
type TelemetryStore = BTreeMap<String, ProjectHistory>;

static TELEMETRY_STORE: OnceLock<RwLock<TelemetryStore>> = OnceLock::new();

fn store() -> &'static RwLock<TelemetryStore> {
    TELEMETRY_STORE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

pub fn ingest_telemetry(
    project_id: &str,
    path_node_id: &str,
    payload: Value,
    received_at_utc: DateTime<Utc>,
) -> Result<(TelemetryObservationView, TelemetryIngestDisposition, usize), String> {
    let project_id = required_text("project_id", project_id)?;
    let path_node_id = required_text("path node_id", path_node_id)?;
    validate_payload(&project_id, &path_node_id, &payload, received_at_utc)?;

    let node_id = required_field(&payload, "node_id")?;
    let sentinel_instance_id = required_field(&payload, "sentinel_instance_id")?;
    let sequence = required_u64_field(&payload, "sequence")?;
    let observed_at_utc = parse_timestamp(&required_field(&payload, "observed_at_utc")?)?;

    let mut guard = store()
        .write()
        .map_err(|error| format!("SmartSentinel telemetry store lock poisoned: {error}"))?;
    let history = guard
        .entry(project_id.clone())
        .or_default()
        .entry(node_id.clone())
        .or_default();

    if let Some(previous) = history.back() {
        if previous.sentinel_instance_id == sentinel_instance_id && sequence <= previous.sequence {
            if sequence == previous.sequence && previous.payload == payload {
                let observation = previous.view(received_at_utc);
                return Ok((
                    observation,
                    TelemetryIngestDisposition::Duplicate,
                    history.len(),
                ));
            }
            return Err(format!(
                "telemetry sequence must increase for node '{}' and sentinel instance '{}'; previous={}, received={}",
                node_id, sentinel_instance_id, previous.sequence, sequence
            ));
        }
    }

    let observation = TelemetryObservation {
        project_id,
        node_id,
        sentinel_instance_id,
        sequence,
        observed_at_utc,
        received_at_utc,
        payload,
    };
    history.push_back(observation);
    while history.len() > DEFAULT_TELEMETRY_HISTORY_CAPACITY {
        history.pop_front();
    }

    let latest = history
        .back()
        .ok_or_else(|| "telemetry history unexpectedly empty after insert".to_string())?
        .view(received_at_utc);
    Ok((
        latest,
        TelemetryIngestDisposition::Inserted,
        history.len(),
    ))
}

pub fn latest_telemetry(
    project_id: &str,
    node_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<TelemetryObservationView>, String> {
    let guard = store()
        .read()
        .map_err(|error| format!("SmartSentinel telemetry store lock poisoned: {error}"))?;
    Ok(guard
        .get(project_id.trim())
        .and_then(|project| project.get(node_id.trim()))
        .and_then(VecDeque::back)
        .map(|observation| observation.view(now)))
}

pub fn latest_telemetry_by_project(
    project_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<TelemetryObservationView>, String> {
    let guard = store()
        .read()
        .map_err(|error| format!("SmartSentinel telemetry store lock poisoned: {error}"))?;
    Ok(guard
        .get(project_id.trim())
        .map(|project| {
            project
                .values()
                .filter_map(VecDeque::back)
                .map(|observation| observation.view(now))
                .collect()
        })
        .unwrap_or_default())
}

pub fn telemetry_history(
    project_id: &str,
    node_id: &str,
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Vec<TelemetryObservationView>, String> {
    let guard = store()
        .read()
        .map_err(|error| format!("SmartSentinel telemetry store lock poisoned: {error}"))?;
    Ok(guard
        .get(project_id.trim())
        .and_then(|project| project.get(node_id.trim()))
        .map(|history| {
            history
                .iter()
                .rev()
                .take(limit)
                .map(|observation| observation.view(now))
                .collect()
        })
        .unwrap_or_default())
}

pub fn telemetry_history_count(project_id: &str, node_id: &str) -> Result<usize, String> {
    let guard = store()
        .read()
        .map_err(|error| format!("SmartSentinel telemetry store lock poisoned: {error}"))?;
    Ok(guard
        .get(project_id.trim())
        .and_then(|project| project.get(node_id.trim()))
        .map(VecDeque::len)
        .unwrap_or(0))
}

pub fn validate_payload(
    project_id: &str,
    path_node_id: &str,
    payload: &Value,
    received_at_utc: DateTime<Utc>,
) -> Result<(), String> {
    if !payload.is_object() {
        return Err("SmartSentinel telemetry payload must be a JSON object".to_string());
    }

    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|error| format!("serialize SmartSentinel telemetry payload: {error}"))?;
    if payload_bytes.len() > MAX_TELEMETRY_PAYLOAD_BYTES {
        return Err(format!(
            "SmartSentinel telemetry payload exceeds {} bytes",
            MAX_TELEMETRY_PAYLOAD_BYTES
        ));
    }

    let schema_version = required_field(payload, "schema_version")?;
    if schema_version != SENTINEL_TELEMETRY_SCHEMA_VERSION {
        return Err(format!(
            "unsupported SmartSentinel telemetry schema_version '{}'; expected '{}'",
            schema_version, SENTINEL_TELEMETRY_SCHEMA_VERSION
        ));
    }

    let node_id = required_field(payload, "node_id")?;
    if node_id != path_node_id {
        return Err(format!(
            "telemetry path node_id '{}' does not match payload node_id '{}'",
            path_node_id, node_id
        ));
    }

    if let Some(payload_project_id) = optional_field(payload, "project_id") {
        if payload_project_id != project_id {
            return Err(format!(
                "telemetry payload project_id '{}' does not match X-Datum-Project '{}'",
                payload_project_id, project_id
            ));
        }
    }

    let _ = required_field(payload, "sentinel_instance_id")?;
    if required_u64_field(payload, "sequence")? == 0 {
        return Err("SmartSentinel telemetry sequence must be greater than zero".to_string());
    }
    let observed_at_utc = parse_timestamp(&required_field(payload, "observed_at_utc")?)?;
    if observed_at_utc > received_at_utc + Duration::seconds(DEFAULT_TELEMETRY_FUTURE_SKEW_SECONDS) {
        return Err(format!(
            "SmartSentinel telemetry observed_at_utc is beyond the tolerated {} second future skew",
            DEFAULT_TELEMETRY_FUTURE_SKEW_SECONDS
        ));
    }

    if payload.get("side_effects_performed").and_then(Value::as_bool) != Some(false) {
        return Err("SmartSentinel telemetry must declare side_effects_performed=false".to_string());
    }
    if payload.get("safe_to_auto_execute").and_then(Value::as_bool) != Some(false) {
        return Err("SmartSentinel telemetry must declare safe_to_auto_execute=false".to_string());
    }
    if !payload.get("host").is_some_and(Value::is_object) {
        return Err("SmartSentinel telemetry host must be a JSON object".to_string());
    }

    if let Some(usage) = path_value(payload, &["host", "cpu", "usage_percent"]) {
        let usage = usage
            .as_f64()
            .ok_or_else(|| "SmartSentinel telemetry host.cpu.usage_percent must be numeric".to_string())?;
        if !usage.is_finite() || !(0.0..=100.0).contains(&usage) {
            return Err("SmartSentinel telemetry host.cpu.usage_percent must be between 0 and 100".to_string());
        }
    }

    Ok(())
}

fn path_value<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(*key))
}

fn required_field(payload: &Value, field: &str) -> Result<String, String> {
    optional_field(payload, field)
        .ok_or_else(|| format!("SmartSentinel telemetry field '{}' must not be empty", field))
}

fn optional_field(payload: &Value, field: &str) -> Option<String> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn required_u64_field(payload: &Value, field: &str) -> Result<u64, String> {
    payload
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("SmartSentinel telemetry field '{}' must be a positive integer", field))
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| format!("SmartSentinel telemetry observed_at_utc must be RFC3339: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn payload(project: &str, sequence: u64) -> Value {
        json!({
            "schema_version": "0.1.0",
            "project_id": project,
            "node_id": "fog-01",
            "sentinel_instance_id": "fog-01-sentinel",
            "sequence": sequence,
            "observed_at_utc": "2026-09-07T20:00:00Z",
            "host": {
                "cpu": {"usage_percent": 10.0}
            },
            "safe_to_auto_execute": false,
            "side_effects_performed": false
        })
    }

    #[test]
    fn accepts_first_observation_and_bounds_history() {
        let now = Utc.with_ymd_and_hms(2026, 9, 7, 20, 0, 1).unwrap();
        let (view, disposition, count) =
            ingest_telemetry("project-first", "fog-01", payload("project-first", 1), now).unwrap();
        assert_eq!(view.node_id, "fog-01");
        assert_eq!(disposition, TelemetryIngestDisposition::Inserted);
        assert_eq!(count, 1);
    }

    #[test]
    fn rejects_identity_mismatch() {
        let now = Utc.with_ymd_and_hms(2026, 9, 7, 20, 0, 1).unwrap();
        let error = ingest_telemetry("project-identity", "cloud-01", payload("project-identity", 1), now).unwrap_err();
        assert!(error.contains("does not match"));
    }

    #[test]
    fn rejects_non_monotonic_sequence_for_same_instance() {
        let now = Utc.with_ymd_and_hms(2026, 9, 7, 20, 0, 1).unwrap();
        ingest_telemetry("project-sequence", "fog-01", payload("project-sequence", 100), now).unwrap();
        let error = ingest_telemetry("project-sequence", "fog-01", payload("project-sequence", 99), now).unwrap_err();
        assert!(error.contains("sequence must increase"));
    }
}
