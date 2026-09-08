use std::{
    collections::{BTreeMap, VecDeque},
    sync::{OnceLock, RwLock},
};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;

pub const SENTINEL_TELEMETRY_SCHEMA_VERSION: &str = "0.1.0";
pub const DEFAULT_TELEMETRY_HISTORY_CAPACITY: usize = 256;
pub const DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS: i64 = 15;
pub const DEFAULT_TELEMETRY_FUTURE_SKEW_SECONDS: i64 = 60;
pub const MAX_TELEMETRY_PAYLOAD_BYTES: usize = 256 * 1024;
pub const FAILURE_SUSPECT_AFTER_CONSECUTIVE_FAILURES: u32 = 1;
pub const FAILURE_UNAVAILABLE_AFTER_CONSECUTIVE_FAILURES: u32 = 3;
pub const FAILURE_RECOVERY_AFTER_CONSECUTIVE_SUCCESSES: u32 = 3;
pub const FAILURE_MISSING_EVALUATION_INTERVAL_SECONDS: i64 = 5;
pub const FAILURE_TRANSITION_HISTORY_CAPACITY: usize = 32;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FailureStateTransitionView {
    pub from: String,
    pub to: String,
    pub reason: String,
    pub evaluated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FailureStateView {
    pub state: String,
    pub reason: String,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub state_changed_at_utc: String,
    pub last_evaluated_at_utc: String,
    pub transitions: Vec<FailureStateTransitionView>,
}

#[derive(Debug, Clone)]
struct FailureStateRecord {
    state: String,
    reason: String,
    consecutive_failures: u32,
    consecutive_successes: u32,
    state_changed_at_utc: DateTime<Utc>,
    last_evaluated_at_utc: DateTime<Utc>,
    last_evidence_key: Option<String>,
    transitions: VecDeque<FailureStateTransitionView>,
}

type FailureStateStore = BTreeMap<(String, String), FailureStateRecord>;

static FAILURE_STATE_STORE: OnceLock<RwLock<FailureStateStore>> = OnceLock::new();

fn failure_state_store() -> &'static RwLock<FailureStateStore> {
    FAILURE_STATE_STORE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// Advances the node state only when a new observation or a new missing-evidence
/// evaluation interval is seen. Repeated dashboard reads cannot manufacture
/// additional failures or recoveries.
pub fn update_failure_state(
    project_id: &str,
    node_id: &str,
    telemetry_freshness: &str,
    sequence: Option<u64>,
    observed_at_utc: Option<&str>,
    now: DateTime<Utc>,
) -> Result<FailureStateView, String> {
    let project_id = project_id.trim();
    let node_id = node_id.trim();
    if project_id.is_empty() || node_id.is_empty() {
        return Err("failure state requires non-empty project_id and node_id".to_string());
    }

    let is_fresh = telemetry_freshness == "fresh";
    let evidence_key = if let (Some(sequence), Some(observed_at_utc)) = (sequence, observed_at_utc) {
        format!("{telemetry_freshness}:{sequence}:{observed_at_utc}")
    } else {
        let interval = now.timestamp() / FAILURE_MISSING_EVALUATION_INTERVAL_SECONDS;
        format!("{telemetry_freshness}:missing:{interval}")
    };

    let mut guard = failure_state_store()
        .write()
        .map_err(|error| format!("failure state store lock poisoned: {error}"))?;
    let record = guard
        .entry((project_id.to_string(), node_id.to_string()))
        .or_insert_with(|| FailureStateRecord {
            state: "healthy".to_string(),
            reason: "initial_state".to_string(),
            consecutive_failures: 0,
            consecutive_successes: 0,
            state_changed_at_utc: now,
            last_evaluated_at_utc: now,
            last_evidence_key: None,
            transitions: VecDeque::new(),
        });

    if record.last_evidence_key.as_deref() == Some(evidence_key.as_str()) {
        return Ok(failure_state_view(record));
    }

    record.last_evidence_key = Some(evidence_key);
    record.last_evaluated_at_utc = now;

    let previous_state = record.state.clone();
    if is_fresh {
        record.consecutive_failures = 0;
        record.consecutive_successes = record.consecutive_successes.saturating_add(1);

        match record.state.as_str() {
            "suspect" => {
                transition_failure_state(record, "healthy", "fresh_telemetry", now);
            }
            "unavailable" => {
                transition_failure_state(record, "recovering", "first_fresh_observation", now);
            }
            "recovering"
                if record.consecutive_successes >= FAILURE_RECOVERY_AFTER_CONSECUTIVE_SUCCESSES =>
            {
                transition_failure_state(record, "healthy", "recovery_stabilized", now);
            }
            _ => {}
        }
    } else {
        record.consecutive_successes = 0;
        record.consecutive_failures = record.consecutive_failures.saturating_add(1);

        match record.state.as_str() {
            "healthy" if record.consecutive_failures >= FAILURE_SUSPECT_AFTER_CONSECUTIVE_FAILURES => {
                transition_failure_state(record, "suspect", "stale_or_missing_telemetry", now);
            }
            "suspect"
                if record.consecutive_failures >= FAILURE_UNAVAILABLE_AFTER_CONSECUTIVE_FAILURES =>
            {
                transition_failure_state(record, "unavailable", "consecutive_telemetry_failures", now);
            }
            "recovering" => {
                transition_failure_state(record, "unavailable", "recovery_lost", now);
            }
            _ => {}
        }
    }

    if previous_state == record.state {
        record.reason = if is_fresh {
            "fresh_telemetry".to_string()
        } else {
            "stale_or_missing_telemetry".to_string()
        };
    }

    Ok(failure_state_view(record))
}

fn transition_failure_state(
    record: &mut FailureStateRecord,
    to: &str,
    reason: &str,
    now: DateTime<Utc>,
) {
    if record.state == to {
        return;
    }

    let transition = FailureStateTransitionView {
        from: record.state.clone(),
        to: to.to_string(),
        reason: reason.to_string(),
        evaluated_at_utc: now.to_rfc3339(),
    };
    record.transitions.push_back(transition);
    while record.transitions.len() > FAILURE_TRANSITION_HISTORY_CAPACITY {
        record.transitions.pop_front();
    }
    record.state = to.to_string();
    record.reason = reason.to_string();
    record.state_changed_at_utc = now;
}

fn failure_state_view(record: &FailureStateRecord) -> FailureStateView {
    FailureStateView {
        state: record.state.clone(),
        reason: record.reason.clone(),
        consecutive_failures: record.consecutive_failures,
        consecutive_successes: record.consecutive_successes,
        state_changed_at_utc: record.state_changed_at_utc.to_rfc3339(),
        last_evaluated_at_utc: record.last_evaluated_at_utc.to_rfc3339(),
        transitions: record.transitions.iter().cloned().collect(),
    }
}


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

            // The shipped observer keeps a stable logical instance id per node.
            // A later cycle with a new cycle_id is therefore a valid process epoch
            // even when its local sequence starts again at one. Older timestamps or
            // reused cycles remain rejected as replay/non-monotonic evidence.
            let new_cycle = optional_field(&payload, "cycle_id");
            let previous_cycle = optional_field(&previous.payload, "cycle_id");
            let is_new_epoch = sequence <= previous.sequence
                && observed_at_utc > previous.observed_at_utc
                && new_cycle.is_some()
                && new_cycle != previous_cycle;

            if !is_new_epoch {
                return Err(format!(
                    "telemetry sequence must increase for node '{}' and sentinel instance '{}'; previous={}, received={}",
                    node_id, sentinel_instance_id, previous.sequence, sequence
                ));
            }
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

fn required_text(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("SmartSentinel telemetry {label} must not be empty"))
    } else {
        Ok(trimmed.to_string())
    }
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
    use serde_json::json;

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
