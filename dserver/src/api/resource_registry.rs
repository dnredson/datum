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

use crate::core::resource_registry::{
    build_resource_registry_record, derive_resource_identity, ResourceDiscoveryRequest,
    ResourceRegistryRecord,
};

type ApiError = (StatusCode, JsonResponse<Value>);

static RESOURCE_REGISTRY: OnceLock<RwLock<BTreeMap<String, ResourceRegistryRecord>>> =
    OnceLock::new();

fn resource_store() -> &'static RwLock<BTreeMap<String, ResourceRegistryRecord>> {
    RESOURCE_REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
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

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": resources.len(),
        "available_unbound_count": available_unbound_count,
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

fn lock_error<T>(error: std::sync::PoisonError<T>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({
            "error": format!("resource registry state lock poisoned: {error}")
        })),
    )
}
