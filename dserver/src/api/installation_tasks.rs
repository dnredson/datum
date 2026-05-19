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
            annotate_installation_task_policy, build_installation_task_approval,
            derive_installation_tasks_from_plan, evaluate_installation_task_readiness,
            normalize_installation_task, validate_installation_task,
            validate_installation_task_can_be_approved,
            validate_installation_task_status_transition, InstallationTask,
            InstallationTaskApproval, InstallationTaskApprovalList, InstallationTaskList,
            InstallationTaskReadiness, InstallationTaskReadinessList,
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
