use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const MIN_SENTINEL_LEASE_SECONDS: u64 = 1;
pub const MAX_SENTINEL_LEASE_SECONDS: u64 = 3600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SentinelRegistrationRequest {
    pub node_id: String,
    pub instance_id: String,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub lease_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SentinelHeartbeatRequest {
    pub node_id: String,
    pub instance_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SentinelRegistrationRecord {
    pub node_id: String,
    pub instance_id: String,
    pub stage: Option<String>,
    pub capabilities: Vec<String>,
    pub registered_at_utc: String,
    pub last_seen_at_utc: String,
    pub lease_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SentinelLeaseStatus {
    Active,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SentinelRegistrationView {
    pub node_id: String,
    pub instance_id: String,
    pub stage: Option<String>,
    pub capabilities: Vec<String>,
    pub registered_at_utc: String,
    pub last_seen_at_utc: String,
    pub lease_seconds: u64,
    pub status: SentinelLeaseStatus,
    pub deployment_side_effects_performed: bool,
    pub authenticated_identity_attested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SentinelRegistrationList {
    pub count: usize,
    pub active_count: usize,
    pub stale_count: usize,
    pub sentinels: Vec<SentinelRegistrationView>,
    pub deployment_side_effects_performed: bool,
}

pub fn validate_sentinel_registration_request(
    request: &SentinelRegistrationRequest,
) -> Result<(), String> {
    require_identifier("node_id", &request.node_id)?;
    require_identifier("instance_id", &request.instance_id)?;

    if !(MIN_SENTINEL_LEASE_SECONDS..=MAX_SENTINEL_LEASE_SECONDS)
        .contains(&request.lease_seconds)
    {
        return Err(format!(
            "lease_seconds must be between {MIN_SENTINEL_LEASE_SECONDS} and {MAX_SENTINEL_LEASE_SECONDS}"
        ));
    }

    if let Some(stage) = &request.stage {
        if stage.trim().is_empty() {
            return Err("stage must be omitted or a non-empty string".to_string());
        }
    }

    let mut normalized = request
        .capabilities
        .iter()
        .map(|capability| capability.trim())
        .collect::<Vec<_>>();
    if normalized.iter().any(|capability| capability.is_empty()) {
        return Err("capabilities must not contain empty strings".to_string());
    }
    normalized.sort_unstable();
    if normalized.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("capabilities must be unique".to_string());
    }

    Ok(())
}

pub fn validate_sentinel_heartbeat_request(
    request: &SentinelHeartbeatRequest,
) -> Result<(), String> {
    require_identifier("node_id", &request.node_id)?;
    require_identifier("instance_id", &request.instance_id)?;
    Ok(())
}

pub fn build_sentinel_registration_record(
    request: &SentinelRegistrationRequest,
    now: DateTime<Utc>,
) -> Result<SentinelRegistrationRecord, String> {
    validate_sentinel_registration_request(request)?;

    let mut capabilities = request
        .capabilities
        .iter()
        .map(|capability| capability.trim().to_string())
        .collect::<Vec<_>>();
    capabilities.sort();

    let timestamp = now.to_rfc3339();
    Ok(SentinelRegistrationRecord {
        node_id: request.node_id.trim().to_string(),
        instance_id: request.instance_id.trim().to_string(),
        stage: request.stage.as_ref().map(|stage| stage.trim().to_string()),
        capabilities,
        registered_at_utc: timestamp.clone(),
        last_seen_at_utc: timestamp,
        lease_seconds: request.lease_seconds,
    })
}

pub fn refresh_sentinel_registration(
    record: &SentinelRegistrationRecord,
    heartbeat: &SentinelHeartbeatRequest,
    now: DateTime<Utc>,
) -> Result<SentinelRegistrationRecord, String> {
    validate_sentinel_heartbeat_request(heartbeat)?;
    if heartbeat.node_id.trim() != record.node_id {
        return Err(format!(
            "heartbeat node_id '{}' does not match registered node '{}'",
            heartbeat.node_id, record.node_id
        ));
    }
    if heartbeat.instance_id.trim() != record.instance_id {
        return Err(format!(
            "heartbeat instance_id '{}' does not match registered instance '{}' for node '{}'",
            heartbeat.instance_id, record.instance_id, record.node_id
        ));
    }

    let mut refreshed = record.clone();
    refreshed.last_seen_at_utc = now.to_rfc3339();
    Ok(refreshed)
}

pub fn materialize_sentinel_registration(
    record: &SentinelRegistrationRecord,
    now: DateTime<Utc>,
) -> Result<SentinelRegistrationView, String> {
    let last_seen = DateTime::parse_from_rfc3339(&record.last_seen_at_utc)
        .map_err(|error| format!("invalid last_seen_at_utc: {error}"))?
        .with_timezone(&Utc);
    let lease_seconds = i64::try_from(record.lease_seconds)
        .map_err(|_| "lease_seconds does not fit signed duration".to_string())?;
    let expires_at = last_seen + Duration::seconds(lease_seconds);
    let status = if now <= expires_at {
        SentinelLeaseStatus::Active
    } else {
        SentinelLeaseStatus::Stale
    };

    Ok(SentinelRegistrationView {
        node_id: record.node_id.clone(),
        instance_id: record.instance_id.clone(),
        stage: record.stage.clone(),
        capabilities: record.capabilities.clone(),
        registered_at_utc: record.registered_at_utc.clone(),
        last_seen_at_utc: record.last_seen_at_utc.clone(),
        lease_seconds: record.lease_seconds,
        status,
        deployment_side_effects_performed: false,
        authenticated_identity_attested: false,
    })
}

pub fn build_sentinel_registration_list(
    records: &[SentinelRegistrationRecord],
    now: DateTime<Utc>,
) -> Result<SentinelRegistrationList, String> {
    let mut sentinels = records
        .iter()
        .map(|record| materialize_sentinel_registration(record, now))
        .collect::<Result<Vec<_>, _>>()?;
    sentinels.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    let active_count = sentinels
        .iter()
        .filter(|sentinel| sentinel.status == SentinelLeaseStatus::Active)
        .count();
    let stale_count = sentinels.len() - active_count;

    Ok(SentinelRegistrationList {
        count: sentinels.len(),
        active_count,
        stale_count,
        sentinels,
        deployment_side_effects_performed: false,
    })
}

fn require_identifier(field: &str, value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must be a non-empty string"));
    }
    if trimmed.len() > 128 {
        return Err(format!("{field} must not exceed 128 characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 31, 15, 0, 0).unwrap()
    }

    fn request() -> SentinelRegistrationRequest {
        SentinelRegistrationRequest {
            node_id: "fog-01".to_string(),
            instance_id: "fog-instance-1".to_string(),
            stage: Some("fog".to_string()),
            capabilities: vec!["wasmtime".to_string(), "artifact_fetch".to_string()],
            lease_seconds: 30,
        }
    }

    #[test]
    fn registration_builds_active_record() {
        let record = build_sentinel_registration_record(&request(), now()).unwrap();
        let view = materialize_sentinel_registration(&record, now()).unwrap();
        assert_eq!(view.status, SentinelLeaseStatus::Active);
        assert!(!view.authenticated_identity_attested);
        assert!(!view.deployment_side_effects_performed);
    }

    #[test]
    fn stale_after_lease_expiry() {
        let record = build_sentinel_registration_record(&request(), now()).unwrap();
        let view = materialize_sentinel_registration(&record, now() + Duration::seconds(31)).unwrap();
        assert_eq!(view.status, SentinelLeaseStatus::Stale);
    }

    #[test]
    fn heartbeat_refreshes_last_seen() {
        let record = build_sentinel_registration_record(&request(), now()).unwrap();
        let heartbeat = SentinelHeartbeatRequest {
            node_id: "fog-01".to_string(),
            instance_id: "fog-instance-1".to_string(),
        };
        let refreshed = refresh_sentinel_registration(&record, &heartbeat, now() + Duration::seconds(20)).unwrap();
        let view = materialize_sentinel_registration(&refreshed, now() + Duration::seconds(40)).unwrap();
        assert_eq!(view.status, SentinelLeaseStatus::Active);
        assert_eq!(refreshed.registered_at_utc, record.registered_at_utc);
        assert_ne!(refreshed.last_seen_at_utc, record.last_seen_at_utc);
    }

    #[test]
    fn heartbeat_instance_mismatch_is_rejected() {
        let record = build_sentinel_registration_record(&request(), now()).unwrap();
        let heartbeat = SentinelHeartbeatRequest {
            node_id: "fog-01".to_string(),
            instance_id: "other-instance".to_string(),
        };
        assert!(refresh_sentinel_registration(&record, &heartbeat, now()).is_err());
    }

    #[test]
    fn lease_outside_bounds_is_rejected() {
        let mut candidate = request();
        candidate.lease_seconds = 0;
        assert!(validate_sentinel_registration_request(&candidate).is_err());
        candidate.lease_seconds = MAX_SENTINEL_LEASE_SECONDS + 1;
        assert!(validate_sentinel_registration_request(&candidate).is_err());
    }

    #[test]
    fn duplicate_capabilities_are_rejected() {
        let mut candidate = request();
        candidate.capabilities = vec!["wasmtime".to_string(), "wasmtime".to_string()];
        assert!(validate_sentinel_registration_request(&candidate).is_err());
    }

    #[test]
    fn registration_list_counts_active_and_stale() {
        let active = build_sentinel_registration_record(&request(), now()).unwrap();
        let mut stale_request = request();
        stale_request.node_id = "cloud-01".to_string();
        stale_request.instance_id = "cloud-instance-1".to_string();
        stale_request.lease_seconds = 5;
        let stale = build_sentinel_registration_record(&stale_request, now()).unwrap();
        let list = build_sentinel_registration_list(&[active, stale], now() + Duration::seconds(6)).unwrap();
        assert_eq!(list.count, 2);
        assert_eq!(list.active_count, 1);
        assert_eq!(list.stale_count, 1);
        assert!(!list.deployment_side_effects_performed);
    }
}
