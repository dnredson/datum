use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::{
    core::{
        installation_plan::{
            derive_installation_plan_for_node, derive_installation_plan_from_deployment_planning,
            empty_installation_plan,
        },
        installation_tasks::{
            annotate_installation_task_policy, build_assisted_execution_attempt,
            build_assisted_execution_claim, build_assisted_execution_request,
            build_installation_task_approval, derive_assisted_execution_state_summary,
            derive_installation_tasks_from_plan, evaluate_installation_task_readiness,
            normalize_installation_task, validate_assisted_execution_attempt_allowed,
            validate_assisted_execution_claim_allowed,
            validate_assisted_execution_claim_status_transition,
            validate_assisted_execution_request_allowed, validate_installation_task,
            validate_installation_task_can_be_approved,
            validate_installation_task_status_transition, AssistedExecutionAttempt,
            AssistedExecutionAttemptBody, AssistedExecutionAttemptList, AssistedExecutionClaim,
            AssistedExecutionClaimList, AssistedExecutionClaimStatusPatch,
            AssistedExecutionRequest, AssistedExecutionRequestList, AssistedExecutionStateSummary,
            AssistedExecutionStateSummaryList, InstallationTask, InstallationTaskApproval,
            InstallationTaskApprovalList, InstallationTaskList, InstallationTaskReadiness,
            InstallationTaskReadinessList,
        },
        local_mapping_preflight::{
            normalize_local_mapping_preflight_report, validate_local_mapping_preflight_report,
            LocalMappingPreflightReport, LocalMappingPreflightReportList,
        },
        local_mapping_recommendations::{
            derive_local_mapping_recommendations_from_tasks, LocalMappingRecommendation,
            LocalMappingRecommendationList,
        },
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

#[derive(Debug, Deserialize)]
pub struct InstallationTaskStatusPatch {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InstallationTaskApprovalRequest {
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub expires_at_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssistedExecutionRequestBody {
    #[serde(default)]
    pub requested_by: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssistedExecutionClaimBody {
    #[serde(default)]
    pub claimed_by: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub lease_expires_at_utc: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AssistedExecutionClaimStatusPatchBody {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    match project_id {
        Some(p) if !p.is_empty() => Ok(p),
        _ => Err((
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "missing or invalid X-Datum-Project header" })),
        )),
    }
}

fn parse_action_plan_payload(raw: &Value) -> Option<Value> {
    let payload = raw
        .get("fields")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())?;

    serde_json::from_str(payload).ok()
}

async fn get_project_installation_plan(
    state: &AppState,
    project_id: &str,
) -> Result<Value, ApiError> {
    let plan = state
        .firebase
        .get_action_plan_latest_optional(project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(
                    json!({ "error": format!("get_action_plan_latest_optional failed: {}", e) }),
                ),
            )
        })?;

    let deployment_planning = plan
        .as_ref()
        .and_then(parse_action_plan_payload)
        .and_then(|plan| plan.get("deployment_planning").cloned());

    Ok(deployment_planning
        .as_ref()
        .map(derive_installation_plan_from_deployment_planning)
        .unwrap_or_else(empty_installation_plan))
}

async fn get_node_installation_plan(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<Value, ApiError> {
    let plan = state
        .firebase
        .get_action_plan_latest_optional(project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(
                    json!({ "error": format!("get_action_plan_latest_optional failed: {}", e) }),
                ),
            )
        })?;

    let deployment_planning = plan
        .as_ref()
        .and_then(parse_action_plan_payload)
        .and_then(|plan| plan.get("deployment_planning").cloned());

    Ok(deployment_planning
        .as_ref()
        .map(|deployment_planning| derive_installation_plan_for_node(deployment_planning, node_id))
        .unwrap_or_else(empty_installation_plan))
}

fn sort_tasks(tasks: &mut [InstallationTask]) {
    tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
}

fn normalize_task_for_response(
    project_id: &str,
    fallback_node_id: Option<&str>,
    task: InstallationTask,
) -> InstallationTask {
    let node_id = if task.node_id.trim().is_empty() {
        fallback_node_id.unwrap_or("").to_string()
    } else {
        task.node_id.clone()
    };

    normalize_installation_task(project_id, &node_id, task)
}

fn task_list(
    project_id: String,
    node_id: Option<String>,
    mut tasks: Vec<InstallationTask>,
) -> InstallationTaskList {
    tasks = tasks
        .into_iter()
        .map(|task| normalize_task_for_response(&project_id, node_id.as_deref(), task))
        .collect();
    sort_tasks(&mut tasks);
    InstallationTaskList {
        project_id,
        node_id,
        count: tasks.len() as u64,
        tasks,
    }
}

fn readiness_list(
    project_id: String,
    node_id: String,
    mut readiness: Vec<InstallationTaskReadiness>,
) -> InstallationTaskReadinessList {
    readiness.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    InstallationTaskReadinessList {
        project_id,
        node_id,
        count: readiness.len() as u64,
        readiness,
    }
}

fn assisted_execution_request_list(
    project_id: String,
    node_id: String,
    mut requests: Vec<AssistedExecutionRequest>,
) -> AssistedExecutionRequestList {
    requests.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    AssistedExecutionRequestList {
        project_id,
        node_id: Some(node_id),
        count: requests.len() as u64,
        requests,
    }
}

fn assisted_execution_claim_list(
    project_id: String,
    node_id: String,
    mut claims: Vec<AssistedExecutionClaim>,
) -> AssistedExecutionClaimList {
    claims.sort_by(|a, b| {
        a.task_id
            .cmp(&b.task_id)
            .then_with(|| a.request_id.cmp(&b.request_id))
    });
    AssistedExecutionClaimList {
        project_id,
        node_id: Some(node_id),
        count: claims.len() as u64,
        claims,
    }
}

fn assisted_execution_attempt_list(
    project_id: String,
    node_id: String,
    mut attempts: Vec<AssistedExecutionAttempt>,
) -> AssistedExecutionAttemptList {
    attempts.sort_by(|a, b| {
        a.task_id
            .cmp(&b.task_id)
            .then_with(|| a.request_id.cmp(&b.request_id))
            .then_with(|| a.claim_id.cmp(&b.claim_id))
    });
    AssistedExecutionAttemptList {
        project_id,
        node_id: Some(node_id),
        count: attempts.len() as u64,
        attempts,
    }
}

fn assisted_execution_state_summary_list(
    project_id: String,
    node_id: String,
    mut summaries: Vec<AssistedExecutionStateSummary>,
) -> AssistedExecutionStateSummaryList {
    summaries.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    AssistedExecutionStateSummaryList {
        project_id,
        node_id: Some(node_id),
        count: summaries.len() as u64,
        summaries,
    }
}

fn local_mapping_recommendation_list(
    project_id: String,
    node_id: Option<String>,
    recommendations: Vec<LocalMappingRecommendation>,
) -> LocalMappingRecommendationList {
    LocalMappingRecommendationList {
        project_id,
        node_id,
        count: recommendations.len() as u64,
        recommendations,
    }
}

fn local_mapping_preflight_report_list(
    project_id: String,
    node_id: Option<String>,
    mut reports: Vec<LocalMappingPreflightReport>,
) -> LocalMappingPreflightReportList {
    reports.sort_by(|a, b| {
        a.node_id
            .cmp(&b.node_id)
            .then_with(|| a.reported_at_utc.cmp(&b.reported_at_utc))
    });
    LocalMappingPreflightReportList {
        project_id,
        node_id,
        count: reports.len() as u64,
        reports,
    }
}

async fn load_assisted_execution_state_summary_for_task(
    state: &AppState,
    project_id: &str,
    node_id: &str,
    task_id: &str,
) -> Result<AssistedExecutionStateSummary, ApiError> {
    let Some(task) = state
        .firebase
        .get_installation_task(project_id, node_id, task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    let normalized_task = normalize_task_for_response(project_id, Some(node_id), task);
    let latest_approval = state
        .firebase
        .get_latest_installation_task_approval(project_id, node_id, task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let readiness = evaluate_installation_task_readiness(
        project_id,
        node_id,
        &normalized_task,
        latest_approval.as_ref(),
    );
    let latest_request = state
        .firebase
        .get_latest_assisted_execution_request(project_id, node_id, task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let latest_claim = match latest_request.as_ref() {
        Some(request) => state
            .firebase
            .get_latest_assisted_execution_claim(project_id, node_id, task_id, &request.request_id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    JsonResponse(json!({ "error": e.to_string() })),
                )
            })?,
        None => None,
    };
    let latest_attempt = match (latest_request.as_ref(), latest_claim.as_ref()) {
        (Some(request), Some(claim)) => state
            .firebase
            .get_latest_assisted_execution_attempt(
                project_id,
                node_id,
                task_id,
                &request.request_id,
                &claim.claim_id,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    JsonResponse(json!({ "error": e.to_string() })),
                )
            })?,
        _ => None,
    };

    Ok(derive_assisted_execution_state_summary(
        project_id,
        node_id,
        Some(&normalized_task),
        latest_approval.as_ref(),
        Some(&readiness),
        latest_request.as_ref(),
        latest_claim.as_ref(),
        latest_attempt.as_ref(),
    ))
}

fn merge_status_reason(task: &mut InstallationTask, reason: Option<&str>) {
    let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };

    let mut metadata = task.metadata.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("status_reason".to_string(), json!(reason));
    }
    task.metadata = Some(metadata);
}

pub async fn materialize_installation_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let installation_plan = get_project_installation_plan(&state, &project_id).await?;
    let tasks = derive_installation_tasks_from_plan(&project_id, None, &installation_plan);

    for task in &tasks {
        validate_installation_task(task).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": err })),
            )
        })?;

        state
            .firebase
            .put_installation_task(&project_id, &task.node_id, task)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    JsonResponse(json!({ "error": e.to_string() })),
                )
            })?;
    }

    Ok(JsonResponse(task_list(project_id, None, tasks)))
}

pub async fn materialize_installation_tasks_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let installation_plan = get_node_installation_plan(&state, &project_id, &node_id).await?;
    let tasks =
        derive_installation_tasks_from_plan(&project_id, Some(&node_id), &installation_plan);

    for task in &tasks {
        validate_installation_task(task).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": err })),
            )
        })?;

        state
            .firebase
            .put_installation_task(&project_id, &task.node_id, task)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    JsonResponse(json!({ "error": e.to_string() })),
                )
            })?;
    }

    Ok(JsonResponse(task_list(project_id, Some(node_id), tasks)))
}

pub async fn list_installation_tasks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let tasks = state
        .firebase
        .list_installation_tasks(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(task_list(project_id, None, tasks)))
}

pub async fn list_installation_tasks_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let tasks = state
        .firebase
        .list_installation_tasks_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(task_list(project_id, Some(node_id), tasks)))
}

pub async fn list_local_mapping_recommendations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<LocalMappingRecommendationList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let tasks = state
        .firebase
        .list_installation_tasks(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    let normalized_tasks = tasks
        .into_iter()
        .map(|task| normalize_task_for_response(&project_id, None, task))
        .collect::<Vec<_>>();
    let recommendations =
        derive_local_mapping_recommendations_from_tasks(&project_id, None, &normalized_tasks);

    Ok(JsonResponse(local_mapping_recommendation_list(
        project_id,
        None,
        recommendations,
    )))
}

pub async fn list_local_mapping_recommendations_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<LocalMappingRecommendationList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let tasks = state
        .firebase
        .list_installation_tasks_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    let normalized_tasks = tasks
        .into_iter()
        .map(|task| normalize_task_for_response(&project_id, Some(&node_id), task))
        .collect::<Vec<_>>();
    let recommendations = derive_local_mapping_recommendations_from_tasks(
        &project_id,
        Some(&node_id),
        &normalized_tasks,
    );

    Ok(JsonResponse(local_mapping_recommendation_list(
        project_id,
        Some(node_id),
        recommendations,
    )))
}

pub async fn post_local_mapping_preflight_report(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<LocalMappingPreflightReport>,
) -> Result<JsonResponse<LocalMappingPreflightReport>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let normalized = normalize_local_mapping_preflight_report(&project_id, &node_id, payload);

    validate_local_mapping_preflight_report(&normalized).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    state
        .firebase
        .put_local_mapping_preflight_report(&project_id, &node_id, &normalized)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(normalized))
}

pub async fn get_latest_local_mapping_preflight_report(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<LocalMappingPreflightReport>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let report = state
        .firebase
        .get_latest_local_mapping_preflight_report(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match report {
        Some(report) => Ok(JsonResponse(report)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "local mapping preflight report not found for project '{}' node '{}'",
                    project_id, node_id
                )
            })),
        )),
    }
}

pub async fn list_latest_local_mapping_preflight_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<LocalMappingPreflightReportList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let reports = state
        .firebase
        .list_latest_local_mapping_preflight_reports(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(local_mapping_preflight_report_list(
        project_id, None, reports,
    )))
}

pub async fn get_installation_task(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTask>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let task = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match task {
        Some(task) => Ok(JsonResponse(normalize_task_for_response(
            &project_id,
            Some(&node_id),
            task,
        ))),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        )),
    }
}

pub async fn patch_installation_task_status(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<InstallationTaskStatusPatch>,
) -> Result<JsonResponse<InstallationTask>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let Some(current_task) = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    validate_installation_task_status_transition(&current_task.status, &payload.status).map_err(
        |err| {
            (
                StatusCode::BAD_REQUEST,
                JsonResponse(json!({ "error": err })),
            )
        },
    )?;

    let mut updated_task = normalize_installation_task(&project_id, &node_id, current_task);
    updated_task.status = payload.status.trim().to_string();
    updated_task.updated_at_utc = Utc::now().to_rfc3339();
    annotate_installation_task_policy(&mut updated_task);
    merge_status_reason(&mut updated_task, payload.reason.as_deref());

    validate_installation_task(&updated_task).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    state
        .firebase
        .put_installation_task(&project_id, &node_id, &updated_task)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(updated_task))
}

pub async fn post_installation_task_approval(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<InstallationTaskApprovalRequest>,
) -> Result<JsonResponse<InstallationTaskApproval>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let Some(task) = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    let normalized_task = normalize_task_for_response(&project_id, Some(&node_id), task);
    validate_installation_task_can_be_approved(&normalized_task).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    let approval = build_installation_task_approval(
        &project_id,
        &node_id,
        &normalized_task,
        payload.approved_by,
        payload.reason,
        payload.expires_at_utc,
    );

    state
        .firebase
        .put_installation_task_approval(&project_id, &node_id, &task_id, &approval)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(approval))
}

pub async fn get_latest_installation_task_approval(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskApproval>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let approval = state
        .firebase
        .get_latest_installation_task_approval(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match approval {
        Some(approval) => Ok(JsonResponse(approval)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task approval not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        )),
    }
}

pub async fn get_installation_task_readiness(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskReadiness>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let task = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    let Some(task) = task else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    let normalized_task = normalize_task_for_response(&project_id, Some(&node_id), task);
    let latest_approval = state
        .firebase
        .get_latest_installation_task_approval(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(evaluate_installation_task_readiness(
        &project_id,
        &node_id,
        &normalized_task,
        latest_approval.as_ref(),
    )))
}

pub async fn post_assisted_execution_request(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<AssistedExecutionRequestBody>,
) -> Result<JsonResponse<AssistedExecutionRequest>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let Some(task) = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    let normalized_task = normalize_task_for_response(&project_id, Some(&node_id), task);
    let latest_approval = state
        .firebase
        .get_latest_installation_task_approval(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let readiness = evaluate_installation_task_readiness(
        &project_id,
        &node_id,
        &normalized_task,
        latest_approval.as_ref(),
    );

    validate_assisted_execution_request_allowed(
        &normalized_task,
        &readiness,
        latest_approval.as_ref(),
    )
    .map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    let request = build_assisted_execution_request(
        &project_id,
        &node_id,
        &normalized_task,
        &readiness,
        latest_approval.as_ref(),
        payload.requested_by,
        payload.reason,
    );

    state
        .firebase
        .put_assisted_execution_request(&project_id, &node_id, &task_id, &request)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(request))
}

pub async fn get_latest_assisted_execution_request(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionRequest>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let request = state
        .firebase
        .get_latest_assisted_execution_request(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match request {
        Some(request) => Ok(JsonResponse(request)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution request not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        )),
    }
}

pub async fn post_assisted_execution_claim(
    State(state): State<AppState>,
    Path((node_id, task_id, request_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(payload): Json<AssistedExecutionClaimBody>,
) -> Result<JsonResponse<AssistedExecutionClaim>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let Some(task) = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    let Some(request) = state
        .firebase
        .get_assisted_execution_request(&project_id, &node_id, &task_id, &request_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution request not found for project '{}' node '{}' task '{}' request '{}'",
                    project_id, node_id, task_id, request_id
                )
            })),
        ));
    };

    let normalized_task = normalize_task_for_response(&project_id, Some(&node_id), task);
    let latest_claim = state
        .firebase
        .get_latest_assisted_execution_claim(&project_id, &node_id, &task_id, &request_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    validate_assisted_execution_claim_allowed(&normalized_task, &request, latest_claim.as_ref())
        .map_err(|err| {
            (
                StatusCode::BAD_REQUEST,
                JsonResponse(json!({ "error": err })),
            )
        })?;

    let claim = build_assisted_execution_claim(
        &project_id,
        &node_id,
        &normalized_task,
        &request,
        payload.claimed_by,
        payload.reason,
        payload.lease_expires_at_utc,
    );

    state
        .firebase
        .put_assisted_execution_claim(&project_id, &node_id, &task_id, &request_id, &claim)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(claim))
}

pub async fn get_latest_assisted_execution_claim(
    State(state): State<AppState>,
    Path((node_id, task_id, request_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionClaim>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let claim = state
        .firebase
        .get_latest_assisted_execution_claim(&project_id, &node_id, &task_id, &request_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match claim {
        Some(claim) => Ok(JsonResponse(claim)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution claim not found for project '{}' node '{}' task '{}' request '{}'",
                    project_id, node_id, task_id, request_id
                )
            })),
        )),
    }
}

pub async fn patch_assisted_execution_claim_status(
    State(state): State<AppState>,
    Path((node_id, task_id, request_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(payload): Json<AssistedExecutionClaimStatusPatchBody>,
) -> Result<JsonResponse<AssistedExecutionClaim>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let Some(current_claim) = state
        .firebase
        .get_latest_assisted_execution_claim(&project_id, &node_id, &task_id, &request_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution claim not found for project '{}' node '{}' task '{}' request '{}'",
                    project_id, node_id, task_id, request_id
                )
            })),
        ));
    };

    let patch = AssistedExecutionClaimStatusPatch {
        status: payload.status,
        reason: payload.reason,
    };

    validate_assisted_execution_claim_status_transition(
        &current_claim.claim_status,
        patch.status.trim(),
    )
    .map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    let updated_claim = state
        .firebase
        .update_assisted_execution_claim_status(
            &project_id,
            &node_id,
            &task_id,
            &request_id,
            patch.status.trim(),
            patch.reason.as_deref(),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!(
                        "assisted execution claim not found for project '{}' node '{}' task '{}' request '{}'",
                        project_id, node_id, task_id, request_id
                    )
                })),
            )
        })?;

    Ok(JsonResponse(updated_claim))
}

pub async fn post_assisted_execution_attempt(
    State(state): State<AppState>,
    Path((node_id, task_id, request_id, claim_id)): Path<(String, String, String, String)>,
    headers: HeaderMap,
    Json(payload): Json<AssistedExecutionAttemptBody>,
) -> Result<JsonResponse<AssistedExecutionAttempt>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let Some(task) = state
        .firebase
        .get_installation_task(&project_id, &node_id, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "installation task not found for project '{}' node '{}' task '{}'",
                    project_id, node_id, task_id
                )
            })),
        ));
    };

    let Some(request) = state
        .firebase
        .get_assisted_execution_request(&project_id, &node_id, &task_id, &request_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution request not found for project '{}' node '{}' task '{}' request '{}'",
                    project_id, node_id, task_id, request_id
                )
            })),
        ));
    };

    let Some(claim) = state
        .firebase
        .get_assisted_execution_claim(&project_id, &node_id, &task_id, &request_id, &claim_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution claim not found for project '{}' node '{}' task '{}' request '{}' claim '{}'",
                    project_id, node_id, task_id, request_id, claim_id
                )
            })),
        ));
    };

    let normalized_task = normalize_task_for_response(&project_id, Some(&node_id), task);

    validate_assisted_execution_attempt_allowed(
        &normalized_task,
        &request,
        &claim,
        payload.execution_mode.trim(),
    )
    .map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    let attempt = build_assisted_execution_attempt(
        &project_id,
        &node_id,
        &normalized_task,
        &request,
        &claim,
        payload.execution_mode.trim(),
        payload.attempted_by,
        payload.message,
    );

    state
        .firebase
        .put_assisted_execution_attempt(
            &project_id,
            &node_id,
            &task_id,
            &request_id,
            &claim_id,
            &attempt,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(attempt))
}

pub async fn get_latest_assisted_execution_attempt(
    State(state): State<AppState>,
    Path((node_id, task_id, request_id, claim_id)): Path<(String, String, String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionAttempt>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let attempt = state
        .firebase
        .get_latest_assisted_execution_attempt(
            &project_id,
            &node_id,
            &task_id,
            &request_id,
            &claim_id,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match attempt {
        Some(attempt) => Ok(JsonResponse(attempt)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "assisted execution attempt not found for project '{}' node '{}' task '{}' request '{}' claim '{}'",
                    project_id, node_id, task_id, request_id, claim_id
                )
            })),
        )),
    }
}

pub async fn get_installation_task_assisted_execution_state(
    State(state): State<AppState>,
    Path((node_id, task_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionStateSummary>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let summary =
        load_assisted_execution_state_summary_for_task(&state, &project_id, &node_id, &task_id)
            .await?;

    Ok(JsonResponse(summary))
}

pub async fn list_assisted_execution_attempts_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionAttemptList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let attempts = state
        .firebase
        .list_assisted_execution_attempts_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(assisted_execution_attempt_list(
        project_id, node_id, attempts,
    )))
}

pub async fn list_assisted_execution_state_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionStateSummaryList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let tasks = state
        .firebase
        .list_installation_tasks_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let approvals = state
        .firebase
        .list_installation_task_approvals_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let requests = state
        .firebase
        .list_assisted_execution_requests_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let claims = state
        .firebase
        .list_assisted_execution_claims_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let attempts = state
        .firebase
        .list_assisted_execution_attempts_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    let approvals_by_task_id = approvals
        .into_iter()
        .map(|approval| (approval.task_id.clone(), approval))
        .collect::<HashMap<_, _>>();
    let requests_by_task_id = requests
        .into_iter()
        .map(|request| (request.task_id.clone(), request))
        .collect::<HashMap<_, _>>();
    let claims_by_identity = claims
        .into_iter()
        .map(|claim| ((claim.task_id.clone(), claim.request_id.clone()), claim))
        .collect::<HashMap<_, _>>();
    let attempts_by_identity = attempts
        .into_iter()
        .map(|attempt| {
            (
                (
                    attempt.task_id.clone(),
                    attempt.request_id.clone(),
                    attempt.claim_id.clone(),
                ),
                attempt,
            )
        })
        .collect::<HashMap<_, _>>();

    let summaries = tasks
        .into_iter()
        .map(|task| normalize_task_for_response(&project_id, Some(&node_id), task))
        .map(|task| {
            let approval = approvals_by_task_id.get(&task.task_id);
            let readiness =
                evaluate_installation_task_readiness(&project_id, &node_id, &task, approval);
            let request = requests_by_task_id.get(&task.task_id);
            let claim = request.and_then(|request| {
                claims_by_identity.get(&(task.task_id.clone(), request.request_id.clone()))
            });
            let latest_attempt = match (request, claim) {
                (Some(request), Some(claim)) => attempts_by_identity.get(&(
                    task.task_id.clone(),
                    request.request_id.clone(),
                    claim.claim_id.clone(),
                )),
                _ => None,
            };

            derive_assisted_execution_state_summary(
                &project_id,
                &node_id,
                Some(&task),
                approval,
                Some(&readiness),
                request,
                claim,
                latest_attempt,
            )
        })
        .collect::<Vec<_>>();

    Ok(JsonResponse(assisted_execution_state_summary_list(
        project_id, node_id, summaries,
    )))
}

pub async fn list_installation_task_approvals_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskApprovalList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let mut approvals = state
        .firebase
        .list_installation_task_approvals_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    approvals.sort_by(|a, b| a.task_id.cmp(&b.task_id));

    Ok(JsonResponse(InstallationTaskApprovalList {
        project_id,
        node_id,
        count: approvals.len() as u64,
        approvals,
    }))
}

pub async fn list_installation_task_readiness_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<InstallationTaskReadinessList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let tasks = state
        .firebase
        .list_installation_tasks_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let approvals = state
        .firebase
        .list_installation_task_approvals_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;
    let approvals_by_task_id = approvals
        .into_iter()
        .map(|approval| (approval.task_id.clone(), approval))
        .collect::<HashMap<_, _>>();

    let readiness = tasks
        .into_iter()
        .map(|task| normalize_task_for_response(&project_id, Some(&node_id), task))
        .map(|task| {
            let approval = approvals_by_task_id.get(&task.task_id);
            evaluate_installation_task_readiness(&project_id, &node_id, &task, approval)
        })
        .collect::<Vec<_>>();

    Ok(JsonResponse(readiness_list(project_id, node_id, readiness)))
}

pub async fn list_assisted_execution_requests_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionRequestList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let requests = state
        .firebase
        .list_assisted_execution_requests_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(assisted_execution_request_list(
        project_id, node_id, requests,
    )))
}

pub async fn list_assisted_execution_claims_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<AssistedExecutionClaimList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let claims = state
        .firebase
        .list_assisted_execution_claims_for_node(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(assisted_execution_claim_list(
        project_id, node_id, claims,
    )))
}
