use std::{
    collections::BTreeMap,
    sync::{OnceLock, RwLock},
};

use axum::{
    extract::{Json, Path},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::Utc;
use serde_json::{json, Value};

use crate::core::{
    resource_binding::{
        build_initial_binding, BindResourceRequest, ResourceRoleBindingRecord,
    },
    resource_registry::{
        build_resource_registry_record, derive_resource_identity, ResourceDiscoveryRequest,
        ResourceRegistryRecord,
    },
};

type ApiError = (StatusCode, JsonResponse<Value>);

static RESOURCE_REGISTRY: OnceLock<RwLock<BTreeMap<String, ResourceRegistryRecord>>> =
    OnceLock::new();
static RESOURCE_BINDINGS: OnceLock<RwLock<BTreeMap<String, ResourceRoleBindingRecord>>> =
    OnceLock::new();

fn resource_store() -> &'static RwLock<BTreeMap<String, ResourceRegistryRecord>> {
    RESOURCE_REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn binding_store() -> &'static RwLock<BTreeMap<String, ResourceRoleBindingRecord>> {
    RESOURCE_BINDINGS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

pub async fn post_resource_discovery(
    headers: HeaderMap,
    Json(payload): Json<ResourceDiscoveryRequest>,
) -> Result<JsonResponse<ResourceRegistryRecord>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let (resource_id, _, _) = derive_resource_identity(&payload).map_err(bad_request)?;
    let key = registry_key(&project_id, &resource_id);
    let now = Utc::now();

    let record = {
        let mut store = resource_store().write().map_err(lock_error)?;
        let existing = store.get(&key).cloned();
        let record = build_resource_registry_record(
            &project_id,
            payload,
            existing.as_ref(),
            now,
        )
        .map_err(bad_request)?;
        store.insert(key, record.clone());
        record
    };

    Ok(JsonResponse(record))
}

pub async fn post_resource_binding(
    Path(resource_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<BindResourceRequest>,
) -> Result<JsonResponse<ResourceRoleBindingRecord>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let resource_id = resource_id.trim();
    if resource_id.is_empty() {
        return Err(bad_request("resource_id must not be empty".to_string()));
    }
    let node_id = payload.node_id.trim();
    if node_id.is_empty() {
        return Err(bad_request("resource binding node_id must not be empty".to_string()));
    }

    let resource_key = registry_key(&project_id, resource_id);
    let mut resources = resource_store().write().map_err(lock_error)?;
    let resource = resources.get_mut(&resource_key).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "resource '{}' not found for project '{}'",
                    resource_id, project_id
                )
            })),
        )
    })?;

    let mut bindings = binding_store().write().map_err(lock_error)?;

    if let Some(assigned) = resource.assigned_node_id.as_deref() {
        if assigned != node_id {
            return Err(conflict(format!(
                "resource '{}' is already bound to node '{}'",
                resource_id, assigned
            )));
        }
        if let Some(existing) = bindings.values().find(|binding| {
            binding.project_id == project_id
                && binding.resource_id == resource_id
                && binding.node_id == node_id
                && binding.status == "active"
        }) {
            return Ok(JsonResponse(existing.clone()));
        }
        return Err(internal_error(format!(
            "resource '{}' claims node '{}' but active binding history is missing",
            resource_id, node_id
        )));
    }

    if let Some(existing) = bindings.values().find(|binding| {
        binding.project_id == project_id
            && binding.resource_id == resource_id
            && binding.status == "active"
    }) {
        return Err(internal_error(format!(
            "resource '{}' has active binding '{}' but registry state is unbound",
            resource_id, existing.binding_id
        )));
    }

    if let Some(existing) = bindings.values().find(|binding| {
        binding.project_id == project_id
            && binding.node_id == node_id
            && binding.status == "active"
    }) {
        return Err(conflict(format!(
            "node '{}' is already bound to resource '{}' by binding '{}'",
            node_id, existing.resource_id, existing.binding_id
        )));
    }

    let binding = build_initial_binding(&project_id, resource, &payload, Utc::now())
        .map_err(bad_request)?;

    resource.state = "bound".to_string();
    resource.assigned_node_id = Some(binding.node_id.clone());
    let binding_key = binding_key(&project_id, &binding.binding_id);
    bindings.insert(binding_key, binding.clone());

    Ok(JsonResponse(binding))
}

pub async fn get_resource_binding(
    Path(resource_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ResourceRoleBindingRecord>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let resource_id = resource_id.trim();
    let binding = binding_store()
        .read()
        .map_err(lock_error)?
        .values()
        .find(|binding| {
            binding.project_id == project_id
                && binding.resource_id == resource_id
                && binding.status == "active"
        })
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!(
                        "active binding for resource '{}' not found in project '{}'",
                        resource_id, project_id
                    )
                })),
            )
        })?;
    Ok(JsonResponse(binding))
}

pub async fn list_resource_bindings(
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let mut bindings = binding_store()
        .read()
        .map_err(lock_error)?
        .values()
        .filter(|binding| binding.project_id == project_id)
        .cloned()
        .collect::<Vec<_>>();
    bindings.sort_by(|a, b| a.bound_at_utc.cmp(&b.bound_at_utc));
    let active_count = bindings
        .iter()
        .filter(|binding| binding.status == "active")
        .count();

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": bindings.len(),
        "active_count": active_count,
        "bindings": bindings,
    })))
}

pub async fn get_resource(
    Path(resource_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ResourceRegistryRecord>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let key = registry_key(&project_id, resource_id.trim());
    let record = resource_store()
        .read()
        .map_err(lock_error)?
        .get(&key)
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": format!(
                        "resource '{}' not found for project '{}'",
                        resource_id, project_id
                    )
                })),
            )
        })?;
    Ok(JsonResponse(record))
}

pub async fn list_resources(
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let prefix = format!("{}|", project_id.trim());
    let mut resources = resource_store()
        .read()
        .map_err(lock_error)?
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(_, record)| record.clone())
        .collect::<Vec<_>>();
    resources.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));

    let available_unbound_count = resources
        .iter()
        .filter(|record| record.state == "available_unbound" && record.planning_eligible)
        .count();
    let bound_count = resources
        .iter()
        .filter(|record| record.state == "bound" && record.assigned_node_id.is_some())
        .count();

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": resources.len(),
        "available_unbound_count": available_unbound_count,
        "bound_count": bound_count,
        "resources": resources,
    })))
}

pub fn resource_registry_snapshot(project_id: &str) -> Result<Vec<ResourceRegistryRecord>, String> {
    let prefix = format!("{}|", project_id.trim());
    let mut resources = resource_store()
        .read()
        .map_err(|error| format!("resource registry state lock poisoned: {error}"))?
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(_, record)| record.clone())
        .collect::<Vec<_>>();
    resources.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
    Ok(resources)
}

fn registry_key(project_id: &str, resource_id: &str) -> String {
    format!("{}|{}", project_id.trim(), resource_id.trim())
}

fn binding_key(project_id: &str, binding_id: &str) -> String {
    format!("{}|{}", project_id.trim(), binding_id.trim())
}

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                JsonResponse(json!({"error":"missing or invalid X-Datum-Project header"})),
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
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({
            "error": format!("resource registry state lock poisoned: {error}")
        })),
    )
}
