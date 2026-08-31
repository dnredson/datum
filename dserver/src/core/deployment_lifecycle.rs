use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::core::deployment_manifest::{validate_deployment_manifest, DeploymentManifest};

pub const DEPLOYMENT_LIFECYCLE_VERSION: &str = "0.1.0";
pub const MAX_ATTEMPTS_PER_LINEAGE: u32 = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentAttemptStatus {
    Discovered,
    Prepared,
    Executing,
    Executed,
    Failed,
    Blocked,
}

impl DeploymentAttemptStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Executed | Self::Failed | Self::Blocked)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentAttemptEvent {
    pub status: DeploymentAttemptStatus,
    pub observed_at_utc: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentAttemptRecord {
    pub lifecycle_version: String,
    pub attempt_id: String,
    pub root_attempt_id: String,
    pub retry_of_attempt_id: Option<String>,
    pub attempt_number: u32,
    pub manifest_id: String,
    pub project_id: String,
    pub assignment_id: String,
    pub node_id: String,
    pub requested_by: String,
    pub status: DeploymentAttemptStatus,
    pub execution_id: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    pub history: Vec<DeploymentAttemptEvent>,
    pub automatic_retry_performed: bool,
    pub governed_execution_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateDeploymentAttemptRequest {
    pub attempt_id: String,
    pub assignment_id: String,
    pub requested_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitionDeploymentAttemptRequest {
    pub status: DeploymentAttemptStatus,
    #[serde(default)]
    pub execution_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryDeploymentAttemptRequest {
    pub attempt_id: String,
    pub requested_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentAttemptList {
    pub manifest_id: String,
    pub project_id: String,
    pub count: usize,
    pub attempts: Vec<DeploymentAttemptRecord>,
    pub automatic_retry_performed: bool,
    pub governed_execution_performed: bool,
}

pub fn build_initial_attempt(
    manifest: &DeploymentManifest,
    request: &CreateDeploymentAttemptRequest,
    now_utc: &str,
) -> Result<DeploymentAttemptRecord, String> {
    validate_deployment_manifest(manifest)?;
    let attempt_id = normalize_identifier("attempt_id", &request.attempt_id)?;
    let assignment_id = normalize_identifier("assignment_id", &request.assignment_id)?;
    let requested_by = normalize_identifier("requested_by", &request.requested_by)?;
    let assignment = manifest
        .assignments
        .iter()
        .find(|item| item.assignment_id == assignment_id)
        .ok_or_else(|| format!("assignment '{assignment_id}' is not present in active manifest"))?;
    let now_utc = normalize_timestamp(now_utc)?;

    Ok(DeploymentAttemptRecord {
        lifecycle_version: DEPLOYMENT_LIFECYCLE_VERSION.to_string(),
        attempt_id: attempt_id.clone(),
        root_attempt_id: attempt_id,
        retry_of_attempt_id: None,
        attempt_number: 1,
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        assignment_id,
        node_id: assignment.target_node.clone(),
        requested_by,
        status: DeploymentAttemptStatus::Discovered,
        execution_id: None,
        created_at_utc: now_utc.clone(),
        updated_at_utc: now_utc.clone(),
        history: vec![DeploymentAttemptEvent {
            status: DeploymentAttemptStatus::Discovered,
            observed_at_utc: now_utc,
            reason: Some("explicit_attempt_created".to_string()),
        }],
        automatic_retry_performed: false,
        governed_execution_performed: false,
    })
}

pub fn transition_attempt(
    record: &DeploymentAttemptRecord,
    request: &TransitionDeploymentAttemptRequest,
    now_utc: &str,
) -> Result<DeploymentAttemptRecord, String> {
    if record.status.is_terminal() {
        return Err(format!(
            "attempt '{}' is terminal in status '{:?}'",
            record.attempt_id, record.status
        ));
    }
    if !transition_allowed(record.status, request.status) {
        return Err(format!(
            "invalid deployment attempt transition {:?} -> {:?}",
            record.status, request.status
        ));
    }
    if request.status == DeploymentAttemptStatus::Executed && request.execution_id.is_none() {
        return Err("executed transition requires execution_id".to_string());
    }
    if request.status != DeploymentAttemptStatus::Executed && request.execution_id.is_some() {
        return Err("execution_id is accepted only for executed transition".to_string());
    }
    if matches!(request.status, DeploymentAttemptStatus::Failed | DeploymentAttemptStatus::Blocked)
        && request
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err("failed/blocked transition requires reason".to_string());
    }

    let now_utc = normalize_timestamp(now_utc)?;
    let execution_id = request
        .execution_id
        .as_deref()
        .map(|value| normalize_identifier("execution_id", value))
        .transpose()?;
    let reason = request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let mut next = record.clone();
    next.status = request.status;
    next.execution_id = execution_id;
    next.updated_at_utc = now_utc.clone();
    next.history.push(DeploymentAttemptEvent {
        status: request.status,
        observed_at_utc: now_utc,
        reason,
    });
    Ok(next)
}

pub fn build_retry_attempt(
    manifest: &DeploymentManifest,
    previous: &DeploymentAttemptRecord,
    request: &RetryDeploymentAttemptRequest,
    now_utc: &str,
) -> Result<DeploymentAttemptRecord, String> {
    validate_deployment_manifest(manifest)?;
    if !matches!(
        previous.status,
        DeploymentAttemptStatus::Failed | DeploymentAttemptStatus::Blocked
    ) {
        return Err(format!(
            "retry requires previous attempt '{}' to be failed or blocked",
            previous.attempt_id
        ));
    }
    if previous.attempt_number >= MAX_ATTEMPTS_PER_LINEAGE {
        return Err(format!(
            "retry limit reached for lineage '{}': max attempts is {}",
            previous.root_attempt_id, MAX_ATTEMPTS_PER_LINEAGE
        ));
    }
    if previous.manifest_id != manifest.manifest_id || previous.project_id != manifest.project_id {
        return Err("retry source attempt does not belong to active manifest".to_string());
    }

    let attempt_id = normalize_identifier("attempt_id", &request.attempt_id)?;
    if attempt_id == previous.attempt_id {
        return Err("retry attempt_id must differ from previous attempt_id".to_string());
    }
    let requested_by = normalize_identifier("requested_by", &request.requested_by)?;
    let assignment = manifest
        .assignments
        .iter()
        .find(|item| item.assignment_id == previous.assignment_id)
        .ok_or_else(|| {
            format!(
                "retry assignment '{}' is not present in active manifest",
                previous.assignment_id
            )
        })?;
    if assignment.target_node != previous.node_id {
        return Err("retry source node no longer matches active assignment target".to_string());
    }
    let now_utc = normalize_timestamp(now_utc)?;

    Ok(DeploymentAttemptRecord {
        lifecycle_version: DEPLOYMENT_LIFECYCLE_VERSION.to_string(),
        attempt_id,
        root_attempt_id: previous.root_attempt_id.clone(),
        retry_of_attempt_id: Some(previous.attempt_id.clone()),
        attempt_number: previous.attempt_number + 1,
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        assignment_id: previous.assignment_id.clone(),
        node_id: previous.node_id.clone(),
        requested_by,
        status: DeploymentAttemptStatus::Discovered,
        execution_id: None,
        created_at_utc: now_utc.clone(),
        updated_at_utc: now_utc.clone(),
        history: vec![DeploymentAttemptEvent {
            status: DeploymentAttemptStatus::Discovered,
            observed_at_utc: now_utc,
            reason: Some("explicit_retry_created".to_string()),
        }],
        automatic_retry_performed: false,
        governed_execution_performed: false,
    })
}

pub fn validate_attempt_collection(
    manifest: &DeploymentManifest,
    attempts: &[DeploymentAttemptRecord],
) -> Result<(), String> {
    validate_deployment_manifest(manifest)?;
    let mut attempt_ids = HashSet::new();
    let mut execution_ids = HashSet::new();
    let mut by_id = HashMap::new();

    for attempt in attempts {
        validate_attempt_record(manifest, attempt)?;
        if !attempt_ids.insert(attempt.attempt_id.clone()) {
            return Err(format!("duplicate deployment attempt_id '{}'", attempt.attempt_id));
        }
        if let Some(execution_id) = attempt.execution_id.as_ref() {
            if !execution_ids.insert(execution_id.clone()) {
                return Err(format!(
                    "duplicate deployment lifecycle execution_id '{execution_id}'"
                ));
            }
        }
        by_id.insert(attempt.attempt_id.as_str(), attempt);
    }

    for attempt in attempts {
        match attempt.retry_of_attempt_id.as_deref() {
            Some(previous_id) => {
                let previous = by_id.get(previous_id).ok_or_else(|| {
                    format!(
                        "retry attempt '{}' references missing previous attempt '{}'",
                        attempt.attempt_id, previous_id
                    )
                })?;
                if attempt.root_attempt_id != previous.root_attempt_id
                    || attempt.attempt_number != previous.attempt_number + 1
                    || attempt.assignment_id != previous.assignment_id
                    || attempt.node_id != previous.node_id
                {
                    return Err(format!(
                        "retry lineage for attempt '{}' is inconsistent",
                        attempt.attempt_id
                    ));
                }
            }
            None => {
                if attempt.attempt_number != 1 || attempt.root_attempt_id != attempt.attempt_id {
                    return Err(format!(
                        "root attempt '{}' has inconsistent lineage metadata",
                        attempt.attempt_id
                    ));
                }
            }
        }
    }

    Ok(())
}

pub fn build_attempt_list(
    manifest: &DeploymentManifest,
    attempts: &[DeploymentAttemptRecord],
) -> DeploymentAttemptList {
    let mut attempts = attempts.to_vec();
    attempts.sort_by(|a, b| {
        a.root_attempt_id
            .cmp(&b.root_attempt_id)
            .then(a.attempt_number.cmp(&b.attempt_number))
            .then(a.attempt_id.cmp(&b.attempt_id))
    });
    DeploymentAttemptList {
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        count: attempts.len(),
        attempts,
        automatic_retry_performed: false,
        governed_execution_performed: false,
    }
}

fn validate_attempt_record(
    manifest: &DeploymentManifest,
    attempt: &DeploymentAttemptRecord,
) -> Result<(), String> {
    if attempt.lifecycle_version != DEPLOYMENT_LIFECYCLE_VERSION {
        return Err(format!(
            "unsupported deployment lifecycle_version '{}'",
            attempt.lifecycle_version
        ));
    }
    normalize_identifier("attempt_id", &attempt.attempt_id)?;
    normalize_identifier("root_attempt_id", &attempt.root_attempt_id)?;
    normalize_identifier("requested_by", &attempt.requested_by)?;
    if attempt.attempt_number == 0 || attempt.attempt_number > MAX_ATTEMPTS_PER_LINEAGE {
        return Err(format!(
            "attempt_number must be in 1..={MAX_ATTEMPTS_PER_LINEAGE}"
        ));
    }
    if attempt.manifest_id != manifest.manifest_id || attempt.project_id != manifest.project_id {
        return Err(format!(
            "deployment attempt '{}' does not match active manifest/project",
            attempt.attempt_id
        ));
    }
    let assignment = manifest
        .assignments
        .iter()
        .find(|item| item.assignment_id == attempt.assignment_id)
        .ok_or_else(|| {
            format!(
                "deployment attempt '{}' references missing assignment '{}'",
                attempt.attempt_id, attempt.assignment_id
            )
        })?;
    if assignment.target_node != attempt.node_id {
        return Err(format!(
            "deployment attempt '{}' node '{}' does not match assignment target '{}'",
            attempt.attempt_id, attempt.node_id, assignment.target_node
        ));
    }
    if attempt.history.is_empty() || attempt.history[0].status != DeploymentAttemptStatus::Discovered {
        return Err(format!(
            "deployment attempt '{}' history must begin with discovered",
            attempt.attempt_id
        ));
    }
    if attempt.history.last().map(|event| event.status) != Some(attempt.status) {
        return Err(format!(
            "deployment attempt '{}' history does not end at current status",
            attempt.attempt_id
        ));
    }
    if attempt.status == DeploymentAttemptStatus::Executed && attempt.execution_id.is_none() {
        return Err(format!(
            "executed deployment attempt '{}' requires execution_id",
            attempt.attempt_id
        ));
    }
    if attempt.status != DeploymentAttemptStatus::Executed && attempt.execution_id.is_some() {
        return Err(format!(
            "non-executed deployment attempt '{}' cannot retain execution_id",
            attempt.attempt_id
        ));
    }
    if attempt.automatic_retry_performed {
        return Err("automatic_retry_performed must remain false".to_string());
    }
    if attempt.governed_execution_performed {
        return Err("governed_execution_performed must remain false".to_string());
    }
    Ok(())
}

fn transition_allowed(from: DeploymentAttemptStatus, to: DeploymentAttemptStatus) -> bool {
    matches!(
        (from, to),
        (DeploymentAttemptStatus::Discovered, DeploymentAttemptStatus::Prepared)
            | (DeploymentAttemptStatus::Discovered, DeploymentAttemptStatus::Blocked)
            | (DeploymentAttemptStatus::Prepared, DeploymentAttemptStatus::Executing)
            | (DeploymentAttemptStatus::Prepared, DeploymentAttemptStatus::Failed)
            | (DeploymentAttemptStatus::Prepared, DeploymentAttemptStatus::Blocked)
            | (DeploymentAttemptStatus::Executing, DeploymentAttemptStatus::Executed)
            | (DeploymentAttemptStatus::Executing, DeploymentAttemptStatus::Failed)
    )
}

fn normalize_identifier(field: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.len() > 128 {
        return Err(format!("{field} must be at most 128 characters"));
    }
    Ok(value.to_string())
}

fn normalize_timestamp(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("lifecycle timestamp must not be empty".to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::core::deployment_manifest::{
        build_deployment_artifact_identity, DeploymentAssignment, DeploymentExecutionBoundary,
        DeploymentRuntimeLimits, DeploymentStrategy, GraphDistribution, DEPLOYMENT_MANIFEST_VERSION,
    };

    use super::*;

    const SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest-i41".to_string(),
            project_id: "project-i41".to_string(),
            strategy: DeploymentStrategy::Distributed,
            graph_distribution: GraphDistribution::NodeSlice,
            source_dgraph_id: None,
            assignments: vec![DeploymentAssignment {
                assignment_id: "local-work".to_string(),
                module_id: "local-module".to_string(),
                function_name: "run".to_string(),
                artifact: build_deployment_artifact_identity(
                    "local-artifact",
                    "http://127.0.0.1:8090/artifacts/local.wasm",
                    SHA,
                )
                .unwrap(),
                target_node: "local-01".to_string(),
                limits: DeploymentRuntimeLimits::default(),
                metadata: Some(json!({"deployment_stage": 0})),
            }],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    fn initial() -> DeploymentAttemptRecord {
        build_initial_attempt(
            &manifest(),
            &CreateDeploymentAttemptRequest {
                attempt_id: "attempt-1".to_string(),
                assignment_id: "local-work".to_string(),
                requested_by: "operator".to_string(),
            },
            "2026-08-31T16:00:00Z",
        )
        .unwrap()
    }

    #[test]
    fn initial_attempt_is_discovered_and_explicit() {
        let attempt = initial();
        assert_eq!(attempt.status, DeploymentAttemptStatus::Discovered);
        assert_eq!(attempt.attempt_number, 1);
        assert_eq!(attempt.root_attempt_id, "attempt-1");
        assert!(!attempt.automatic_retry_performed);
        assert!(!attempt.governed_execution_performed);
    }

    #[test]
    fn full_success_lifecycle_is_valid() {
        let prepared = transition_attempt(
            &initial(),
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Prepared,
                execution_id: None,
                reason: None,
            },
            "2026-08-31T16:00:01Z",
        )
        .unwrap();
        let executing = transition_attempt(
            &prepared,
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Executing,
                execution_id: None,
                reason: None,
            },
            "2026-08-31T16:00:02Z",
        )
        .unwrap();
        let executed = transition_attempt(
            &executing,
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Executed,
                execution_id: Some("exec-1".to_string()),
                reason: None,
            },
            "2026-08-31T16:00:03Z",
        )
        .unwrap();
        assert_eq!(executed.status, DeploymentAttemptStatus::Executed);
        assert_eq!(executed.execution_id.as_deref(), Some("exec-1"));
        assert_eq!(executed.history.len(), 4);
    }

    #[test]
    fn invalid_state_jump_is_rejected() {
        let error = transition_attempt(
            &initial(),
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Executed,
                execution_id: Some("exec-1".to_string()),
                reason: None,
            },
            "2026-08-31T16:00:03Z",
        )
        .unwrap_err();
        assert!(error.contains("invalid deployment attempt transition"));
    }

    #[test]
    fn terminal_attempt_cannot_transition() {
        let blocked = transition_attempt(
            &initial(),
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Blocked,
                execution_id: None,
                reason: Some("dependency unavailable".to_string()),
            },
            "2026-08-31T16:00:01Z",
        )
        .unwrap();
        assert!(transition_attempt(
            &blocked,
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Prepared,
                execution_id: None,
                reason: None,
            },
            "2026-08-31T16:00:02Z"
        )
        .is_err());
    }

    #[test]
    fn explicit_retry_links_to_failed_attempt() {
        let prepared = transition_attempt(
            &initial(),
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Prepared,
                execution_id: None,
                reason: None,
            },
            "2026-08-31T16:00:01Z",
        )
        .unwrap();
        let failed = transition_attempt(
            &prepared,
            &TransitionDeploymentAttemptRequest {
                status: DeploymentAttemptStatus::Failed,
                execution_id: None,
                reason: Some("preflight failed".to_string()),
            },
            "2026-08-31T16:00:02Z",
        )
        .unwrap();
        let retry = build_retry_attempt(
            &manifest(),
            &failed,
            &RetryDeploymentAttemptRequest {
                attempt_id: "attempt-2".to_string(),
                requested_by: "operator".to_string(),
            },
            "2026-08-31T16:00:03Z",
        )
        .unwrap();
        assert_eq!(retry.retry_of_attempt_id.as_deref(), Some("attempt-1"));
        assert_eq!(retry.root_attempt_id, "attempt-1");
        assert_eq!(retry.attempt_number, 2);
        assert!(!retry.automatic_retry_performed);
    }

    #[test]
    fn retry_before_failure_or_block_is_rejected() {
        let error = build_retry_attempt(
            &manifest(),
            &initial(),
            &RetryDeploymentAttemptRequest {
                attempt_id: "attempt-2".to_string(),
                requested_by: "operator".to_string(),
            },
            "2026-08-31T16:00:03Z",
        )
        .unwrap_err();
        assert!(error.contains("failed or blocked"));
    }

    #[test]
    fn retry_limit_is_enforced() {
        let mut previous = initial();
        previous.status = DeploymentAttemptStatus::Failed;
        previous.attempt_number = MAX_ATTEMPTS_PER_LINEAGE;
        previous.history.push(DeploymentAttemptEvent {
            status: DeploymentAttemptStatus::Failed,
            observed_at_utc: "2026-08-31T16:00:02Z".to_string(),
            reason: Some("failed".to_string()),
        });
        let error = build_retry_attempt(
            &manifest(),
            &previous,
            &RetryDeploymentAttemptRequest {
                attempt_id: "attempt-4".to_string(),
                requested_by: "operator".to_string(),
            },
            "2026-08-31T16:00:03Z",
        )
        .unwrap_err();
        assert!(error.contains("retry limit reached"));
    }

    #[test]
    fn collection_rejects_duplicate_execution_ids() {
        let mut one = initial();
        one.status = DeploymentAttemptStatus::Executed;
        one.execution_id = Some("exec-duplicate".to_string());
        one.history.push(DeploymentAttemptEvent {
            status: DeploymentAttemptStatus::Executed,
            observed_at_utc: "2026-08-31T16:00:03Z".to_string(),
            reason: None,
        });
        let mut two = one.clone();
        two.attempt_id = "attempt-2".to_string();
        two.root_attempt_id = "attempt-2".to_string();
        let error = validate_attempt_collection(&manifest(), &[one, two]).unwrap_err();
        assert!(error.contains("duplicate deployment lifecycle execution_id"));
    }
}
