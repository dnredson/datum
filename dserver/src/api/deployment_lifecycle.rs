use std::sync::{OnceLock, RwLock};

use axum::{
    extract::{Json, Path},
    http::StatusCode,
    Json as JsonResponse,
};
use chrono::Utc;
use serde_json::{json, Value};

use crate::{
    api::controlled_wasm_deployment::{
        controlled_live_reports_snapshot, controlled_manifest_snapshot,
    },
    core::{
        controlled_wasm_deployment::report_execution_id,
        deployment_lifecycle::{
            build_attempt_list, build_initial_attempt, build_retry_attempt, transition_attempt,
            validate_attempt_collection, CreateDeploymentAttemptRequest, DeploymentAttemptList,
            DeploymentAttemptRecord, DeploymentAttemptStatus, RetryDeploymentAttemptRequest,
            TransitionDeploymentAttemptRequest,
        },
    },
};

type ApiError = (StatusCode, JsonResponse<Value>);

static DEPLOYMENT_ATTEMPTS: OnceLock<RwLock<Vec<DeploymentAttemptRecord>>> = OnceLock::new();

fn attempt_store() -> &'static RwLock<Vec<DeploymentAttemptRecord>> {
    DEPLOYMENT_ATTEMPTS.get_or_init(|| RwLock::new(Vec::new()))
}

pub fn attempts_snapshot() -> Result<Vec<DeploymentAttemptRecord>, String> {
    attempt_store()
        .read()
        .map(|guard| guard.clone())
        .map_err(|error| format!("deployment lifecycle attempt lock poisoned: {error}"))
}

pub fn clear_attempts() -> Result<(), String> {
    let mut attempts = attempt_store()
        .write()
        .map_err(|error| format!("deployment lifecycle attempt lock poisoned: {error}"))?;
    attempts.clear();
    Ok(())
}

pub async fn post_deployment_attempt(
    Json(payload): Json<CreateDeploymentAttemptRequest>,
) -> Result<JsonResponse<DeploymentAttemptRecord>, ApiError> {
    let manifest = active_manifest()?;
    let now = Utc::now().to_rfc3339();
    let attempt = build_initial_attempt(&manifest, &payload, &now).map_err(bad_request)?;

    let mut attempts = attempt_store().write().map_err(lock_error)?;
    if attempts
        .iter()
        .any(|existing| existing.attempt_id == attempt.attempt_id)
    {
        return Err(conflict(format!(
            "deployment attempt_id '{}' already exists",
            attempt.attempt_id
        )));
    }
    let mut candidate = attempts.clone();
    candidate.push(attempt.clone());
    validate_attempt_collection(&manifest, &candidate).map_err(bad_request)?;
    *attempts = candidate;

    Ok(JsonResponse(attempt))
}

pub async fn get_deployment_attempt(
    Path(attempt_id): Path<String>,
) -> Result<JsonResponse<DeploymentAttemptRecord>, ApiError> {
    let attempts = attempt_store().read().map_err(lock_error)?;
    attempts
        .iter()
        .find(|attempt| attempt.attempt_id == attempt_id)
        .cloned()
        .map(JsonResponse)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!("deployment attempt '{}' was not found", attempt_id)
                })),
            )
        })
}

pub async fn list_deployment_attempts(
) -> Result<JsonResponse<DeploymentAttemptList>, ApiError> {
    let manifest = active_manifest()?;
    let attempts = attempts_snapshot().map_err(internal_error)?;
    validate_attempt_collection(&manifest, &attempts).map_err(bad_request)?;
    Ok(JsonResponse(build_attempt_list(&manifest, &attempts)))
}

pub async fn post_deployment_attempt_transition(
    Path(attempt_id): Path<String>,
    Json(payload): Json<TransitionDeploymentAttemptRequest>,
) -> Result<JsonResponse<DeploymentAttemptRecord>, ApiError> {
    let manifest = active_manifest()?;
    let reports = controlled_live_reports_snapshot().map_err(internal_error)?;
    let now = Utc::now().to_rfc3339();
    let mut attempts = attempt_store().write().map_err(lock_error)?;
    let index = attempts
        .iter()
        .position(|attempt| attempt.attempt_id == attempt_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!("deployment attempt '{}' was not found", attempt_id)
                })),
            )
        })?;

    let next = transition_attempt(&attempts[index], &payload, &now).map_err(bad_request)?;
    if next.status == DeploymentAttemptStatus::Executed {
        let execution_id = next
            .execution_id
            .as_deref()
            .expect("executed transition validated execution_id");
        let report = reports
            .iter()
            .find(|report| report_execution_id(report) == Some(execution_id))
            .ok_or_else(|| {
                bad_request(format!(
                    "executed lifecycle transition requires existing live report execution_id '{}'",
                    execution_id
                ))
            })?;
        let report_assignment = report
            .get("assignment_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let report_node = report
            .get("node_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if report_assignment != next.assignment_id || report_node != next.node_id {
            return Err(bad_request(format!(
                "live report execution_id '{}' does not match lifecycle assignment/node",
                execution_id
            )));
        }
        if attempts.iter().enumerate().any(|(other_index, attempt)| {
            other_index != index && attempt.execution_id.as_deref() == Some(execution_id)
        }) {
            return Err(conflict(format!(
                "execution_id '{}' is already linked to another deployment attempt",
                execution_id
            )));
        }
    }

    let mut candidate = attempts.clone();
    candidate[index] = next.clone();
    validate_attempt_collection(&manifest, &candidate).map_err(bad_request)?;
    *attempts = candidate;
    Ok(JsonResponse(next))
}

pub async fn post_deployment_attempt_retry(
    Path(attempt_id): Path<String>,
    Json(payload): Json<RetryDeploymentAttemptRequest>,
) -> Result<JsonResponse<DeploymentAttemptRecord>, ApiError> {
    let manifest = active_manifest()?;
    let now = Utc::now().to_rfc3339();
    let mut attempts = attempt_store().write().map_err(lock_error)?;
    let previous = attempts
        .iter()
        .find(|attempt| attempt.attempt_id == attempt_id)
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!("deployment attempt '{}' was not found", attempt_id)
                })),
            )
        })?;
    let retry = build_retry_attempt(&manifest, &previous, &payload, &now).map_err(bad_request)?;
    if attempts
        .iter()
        .any(|existing| existing.attempt_id == retry.attempt_id)
    {
        return Err(conflict(format!(
            "deployment attempt_id '{}' already exists",
            retry.attempt_id
        )));
    }
    let mut candidate = attempts.clone();
    candidate.push(retry.clone());
    validate_attempt_collection(&manifest, &candidate).map_err(bad_request)?;
    *attempts = candidate;
    Ok(JsonResponse(retry))
}

fn active_manifest() -> Result<crate::core::deployment_manifest::DeploymentManifest, ApiError> {
    controlled_manifest_snapshot()
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::PRECONDITION_FAILED,
                JsonResponse(json!({
                    "error": "publish the controlled WASM deployment manifest before lifecycle attempts"
                })),
            )
        })
}

fn bad_request(message: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({"error": message})),
    )
}

fn conflict(message: String) -> ApiError {
    (
        StatusCode::CONFLICT,
        JsonResponse(json!({"error": message})),
    )
}

fn internal_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message})),
    )
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> ApiError {
    internal_error(format!("deployment lifecycle attempt lock poisoned: {error}"))
}
