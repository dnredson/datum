use std::{
    env,
    path::{Path as FsPath, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
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
    resource_registry_persistence::{
        load_resource_registry_persisted_state, persist_resource_registry_state,
        ResourceRegistryPersistedState,
    },
};

type ApiError = (StatusCode, JsonResponse<Value>);

const DEFAULT_RESOURCE_REGISTRY_STATE_PATH: &str = ".datum-resource-registry-state.json";
const RESOURCE_REGISTRY_STATE_PATH_ENV: &str = "DATUM_RESOURCE_REGISTRY_STATE_PATH";

static RESOURCE_STATE: OnceLock<RwLock<ResourceRegistryPersistedState>> = OnceLock::new();
static PERSISTENCE_PATH: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
static STATE_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();
static PERSISTENCE_INITIALIZED: OnceLock<()> = OnceLock::new();

fn resource_state_store() -> &'static RwLock<ResourceRegistryPersistedState> {
    RESOURCE_STATE.get_or_init(|| RwLock::new(ResourceRegistryPersistedState::empty()))
}

fn persistence_path_store() -> &'static RwLock<Option<PathBuf>> {
    PERSISTENCE_PATH.get_or_init(|| RwLock::new(None))
}

fn state_transaction() -> &'static Mutex<()> {
    STATE_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn ensure_resource_registry_persistence_initialized() {
    PERSISTENCE_INITIALIZED.get_or_init(|| {
        let path = env::var(RESOURCE_REGISTRY_STATE_PATH_ENV)
            .unwrap_or_else(|_| DEFAULT_RESOURCE_REGISTRY_STATE_PATH.to_string());
        match initialize_resource_registry_persistence(FsPath::new(&path)) {
            Ok(true) => println!("restored resource registry and bindings from {path}"),
            Ok(false) => {}
            Err(error) => eprintln!(
                "resource registry persisted state was not activated from {path}: {error}; starting resource registry empty"
            ),
        }
    });
}

pub fn initialize_resource_registry_persistence(path: &FsPath) -> Result<bool, String> {
    let _transaction = state_transaction()
        .lock()
        .map_err(|error| format!("resource registry state transaction lock poisoned: {error}"))?;
    {
        let mut configured = persistence_path_store()
            .write()
            .map_err(|error| format!("resource registry persistence path lock poisoned: {error}"))?;
        *configured = Some(path.to_path_buf());
    }
    match load_resource_registry_persisted_state(path) {
        Ok(Some(state)) => {
            replace_in_memory_state_string(&state)?;
            Ok(true)
        }
        Ok(None) => {
            replace_in_memory_state_string(&ResourceRegistryPersistedState::empty())?;
            Ok(false)
        }
        Err(error) => {
            replace_in_memory_state_string(&ResourceRegistryPersistedState::empty())?;
            Err(error)
        }
    }
}

pub async fn post_resource_discovery(
    headers: HeaderMap,
    Json(payload): Json<ResourceDiscoveryRequest>,
) -> Result<JsonResponse<ResourceRegistryRecord>, ApiError> {
    ensure_resource_registry_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let (resource_id, _, _) = derive_resource_identity(&payload).map_err(bad_request)?;
    let key = registry_key(&project_id, &resource_id);
    let now = Utc::now();
    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let mut candidate = resource_state_store().read().map_err(lock_error)?.clone();
    let existing = candidate.resources.get(&key).cloned();
    let record = build_resource_registry_record(&project_id, payload, existing.as_ref(), now)
        .map_err(bad_request)?;
    candidate.resources.insert(key, record.clone());
    persist_candidate(&candidate)?;
    replace_in_memory_state(&candidate)?;
    Ok(JsonResponse(record))
}

pub async fn post_resource_binding(
    Path(resource_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<BindResourceRequest>,
) -> Result<JsonResponse<ResourceRoleBindingRecord>, ApiError> {
    ensure_resource_registry_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let resource_id = resource_id.trim();
    if resource_id.is_empty() {
        return Err(bad_request("resource_id must not be empty".to_string()));
    }
    let node_id = payload.node_id.trim();
    if node_id.is_empty() {
        return Err(bad_request("resource binding node_id must not be empty".to_string()));
    }
    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let mut candidate = resource_state_store().read().map_err(lock_error)?.clone();
    let resource_key = registry_key(&project_id, resource_id);
    let resource = candidate.resources.get(&resource_key).cloned().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            JsonResponse(json!({"error":format!("resource '{}' not found for project '{}'",resource_id,project_id)})),
        )
    })?;
    if let Some(assigned) = resource.assigned_node_id.as_deref() {
        if assigned != node_id {
            return Err(conflict(format!("resource '{}' is already bound to node '{}'", resource_id, assigned)));
        }
        if let Some(existing) = candidate.bindings.values().find(|binding| {
            binding.project_id == project_id && binding.resource_id == resource_id && binding.node_id == node_id && binding.status == "active"
        }) {
            return Ok(JsonResponse(existing.clone()));
        }
        return Err(internal_error(format!("resource '{}' claims node '{}' but active binding history is missing", resource_id, node_id)));
    }
    if let Some(existing) = candidate.bindings.values().find(|binding| binding.project_id == project_id && binding.resource_id == resource_id && binding.status == "active") {
        return Err(internal_error(format!("resource '{}' has active binding '{}' but registry state is unbound", resource_id, existing.binding_id)));
    }
    if let Some(existing) = candidate.bindings.values().find(|binding| binding.project_id == project_id && binding.node_id == node_id && binding.status == "active") {
        return Err(conflict(format!("node '{}' is already bound to resource '{}' by binding '{}'", node_id, existing.resource_id, existing.binding_id)));
    }
    let binding = build_initial_binding(&project_id, &resource, &payload, Utc::now()).map_err(bad_request)?;
    let mutable_resource = candidate.resources.get_mut(&resource_key).expect("candidate resource cloned from same key must still exist");
    mutable_resource.state = "bound".to_string();
    mutable_resource.assigned_node_id = Some(binding.node_id.clone());
    candidate.bindings.insert(binding_key(&project_id, &binding.binding_id), binding.clone());
    persist_candidate(&candidate)?;
    replace_in_memory_state(&candidate)?;
    Ok(JsonResponse(binding))
}

pub async fn get_resource_binding(
    Path(resource_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ResourceRoleBindingRecord>, ApiError> {
    ensure_resource_registry_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let resource_id = resource_id.trim();
    let binding = resource_state_store().read().map_err(lock_error)?.bindings.values().find(|binding| {
        binding.project_id == project_id && binding.resource_id == resource_id && binding.status == "active"
    }).cloned().ok_or_else(|| (
        StatusCode::NOT_FOUND,
        JsonResponse(json!({"error":format!("active binding for resource '{}' not found in project '{}'",resource_id,project_id)})),
    ))?;
    Ok(JsonResponse(binding))
}

pub async fn list_resource_bindings(headers: HeaderMap) -> Result<JsonResponse<Value>, ApiError> {
    ensure_resource_registry_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let mut bindings = resource_state_store().read().map_err(lock_error)?.bindings.values()
        .filter(|binding| binding.project_id == project_id).cloned().collect::<Vec<_>>();
    bindings.sort_by(|a, b| a.bound_at_utc.cmp(&b.bound_at_utc));
    let active_count = bindings.iter().filter(|binding| binding.status == "active").count();
    Ok(JsonResponse(json!({"project_id":project_id,"count":bindings.len(),"active_count":active_count,"bindings":bindings})))
}

pub async fn get_resource(Path(resource_id): Path<String>, headers: HeaderMap) -> Result<JsonResponse<ResourceRegistryRecord>, ApiError> {
    ensure_resource_registry_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let key = registry_key(&project_id, resource_id.trim());
    let record = resource_state_store().read().map_err(lock_error)?.resources.get(&key).cloned().ok_or_else(|| (
        StatusCode::NOT_FOUND,
        JsonResponse(json!({"error":format!("resource '{}' not found for project '{}'",resource_id,project_id)})),
    ))?;
    Ok(JsonResponse(record))
}

pub async fn list_resources(headers: HeaderMap) -> Result<JsonResponse<Value>, ApiError> {
    ensure_resource_registry_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let prefix = format!("{}|", project_id.trim());
    let mut resources = resource_state_store().read().map_err(lock_error)?.resources.iter()
        .filter(|(key, _)| key.starts_with(&prefix)).map(|(_, record)| record.clone()).collect::<Vec<_>>();
    resources.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
    let available_unbound_count = resources.iter().filter(|record| record.state == "available_unbound" && record.planning_eligible).count();
    let bound_count = resources.iter().filter(|record| record.state == "bound" && record.assigned_node_id.is_some()).count();
    Ok(JsonResponse(json!({"project_id":project_id,"count":resources.len(),"available_unbound_count":available_unbound_count,"bound_count":bound_count,"resources":resources})))
}

pub fn resource_registry_snapshot(project_id: &str) -> Result<Vec<ResourceRegistryRecord>, String> {
    ensure_resource_registry_persistence_initialized();
    let prefix = format!("{}|", project_id.trim());
    let mut resources = resource_state_store().read().map_err(|error| format!("resource registry state lock poisoned: {error}"))?.resources.iter()
        .filter(|(key, _)| key.starts_with(&prefix)).map(|(_, record)| record.clone()).collect::<Vec<_>>();
    resources.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
    Ok(resources)
}

pub fn active_resource_binding_for_node(
    project_id: &str,
    node_id: &str,
) -> Result<Option<ResourceRoleBindingRecord>, String> {
    ensure_resource_registry_persistence_initialized();
    let state = resource_state_store()
        .read()
        .map_err(|error| format!("resource registry state lock poisoned: {error}"))?;
    Ok(state.bindings.values().find(|binding| {
        binding.project_id == project_id.trim()
            && binding.node_id == node_id.trim()
            && binding.status == "active"
    }).cloned())
}

fn persist_candidate(candidate: &ResourceRegistryPersistedState) -> Result<(), ApiError> {
    let path = persistence_path_store().read().map_err(lock_error)?.clone()
        .ok_or_else(|| internal_error("resource registry persistence path is not initialized".to_string()))?;
    persist_resource_registry_state(&path, candidate).map_err(internal_error)
}

fn replace_in_memory_state(candidate: &ResourceRegistryPersistedState) -> Result<(), ApiError> {
    let mut state = resource_state_store().write().map_err(lock_error)?;
    *state = candidate.clone();
    Ok(())
}

fn replace_in_memory_state_string(candidate: &ResourceRegistryPersistedState) -> Result<(), String> {
    let mut state = resource_state_store().write().map_err(|error| format!("resource registry state lock poisoned: {error}"))?;
    *state = candidate.clone();
    Ok(())
}

fn registry_key(project_id: &str, resource_id: &str) -> String { format!("{}|{}", project_id.trim(), resource_id.trim()) }
fn binding_key(project_id: &str, binding_id: &str) -> String { format!("{}|{}", project_id.trim(), binding_id.trim()) }

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers.get("x-datum-project").and_then(|value| value.to_str().ok()).map(str::trim)
        .filter(|value| !value.is_empty()).map(ToString::to_string)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, JsonResponse(json!({"error":"missing or invalid X-Datum-Project header"}))))
}
fn bad_request(message: String) -> ApiError { (StatusCode::BAD_REQUEST, JsonResponse(json!({"error":message}))) }
fn conflict(message: String) -> ApiError { (StatusCode::CONFLICT, JsonResponse(json!({"error":message}))) }
fn internal_error(message: String) -> ApiError { (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error":message}))) }
fn lock_error<T>(error: std::sync::PoisonError<T>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error":format!("resource registry state lock poisoned: {error}")})))
}
