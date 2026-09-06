use std::{
    collections::BTreeMap,
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

use crate::{
    api::resource_registry::resource_registry_snapshot,
    core::{
        operational_dgraph::{
            build_operational_slice, build_reconciliation_authorization, validate_operational_dgraph,
            OperationalDGraph, OperationalDGraphSlice, OperationalReconciliationAuthorization,
            OperationalReconciliationAuthorizationRequest,
        },
        operational_dgraph_persistence::{
            load_operational_dgraph_persisted_state, persist_operational_dgraph_state,
            OperationalDGraphPersistedState, OPERATIONAL_DGRAPH_PERSISTENCE_VERSION,
        },
    },
};

type ApiError = (StatusCode, JsonResponse<Value>);

const DEFAULT_OPERATIONAL_DGRAPH_STATE_PATH: &str = ".datum-operational-dgraph-state.json";
const OPERATIONAL_DGRAPH_STATE_PATH_ENV: &str = "DATUM_OPERATIONAL_DGRAPH_STATE_PATH";

static OPERATIONAL_DGRAPHS: OnceLock<RwLock<BTreeMap<String, OperationalDGraph>>> = OnceLock::new();
static OPERATIONAL_AUTHORIZATIONS: OnceLock<
    RwLock<BTreeMap<String, OperationalReconciliationAuthorization>>,
> = OnceLock::new();
static PERSISTENCE_PATH: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
static STATE_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();
static PERSISTENCE_INITIALIZED: OnceLock<()> = OnceLock::new();

fn graph_store() -> &'static RwLock<BTreeMap<String, OperationalDGraph>> {
    OPERATIONAL_DGRAPHS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn authorization_store(
) -> &'static RwLock<BTreeMap<String, OperationalReconciliationAuthorization>> {
    OPERATIONAL_AUTHORIZATIONS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn persistence_path_store() -> &'static RwLock<Option<PathBuf>> {
    PERSISTENCE_PATH.get_or_init(|| RwLock::new(None))
}

fn state_transaction() -> &'static Mutex<()> {
    STATE_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn ensure_operational_dgraph_persistence_initialized() {
    PERSISTENCE_INITIALIZED.get_or_init(|| {
        let path = env::var(OPERATIONAL_DGRAPH_STATE_PATH_ENV)
            .unwrap_or_else(|_| DEFAULT_OPERATIONAL_DGRAPH_STATE_PATH.to_string());
        match initialize_operational_dgraph_persistence(FsPath::new(&path)) {
            Ok(true) => {
                println!("restored operational D-Graph state from {path}");
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "operational D-Graph persisted state was not activated from {path}: {error}; starting operational D-Graph state empty"
                );
            }
        }
    });
}

pub fn initialize_operational_dgraph_persistence(path: &FsPath) -> Result<bool, String> {
    let _transaction = state_transaction()
        .lock()
        .map_err(|error| format!("operational D-Graph state transaction lock poisoned: {error}"))?;

    {
        let mut configured = persistence_path_store().write().map_err(|error| {
            format!("operational D-Graph persistence path lock poisoned: {error}")
        })?;
        *configured = Some(path.to_path_buf());
    }

    match load_operational_dgraph_persisted_state(path) {
        Ok(Some(state)) => {
            replace_in_memory_graphs(&state)?;
            clear_in_memory_authorizations()?;
            Ok(true)
        }
        Ok(None) => {
            replace_in_memory_graphs(&OperationalDGraphPersistedState::empty())?;
            clear_in_memory_authorizations()?;
            Ok(false)
        }
        Err(error) => {
            replace_in_memory_graphs(&OperationalDGraphPersistedState::empty())?;
            clear_in_memory_authorizations()?;
            Err(error)
        }
    }
}

pub async fn put_operational_dgraph(
    headers: HeaderMap,
    Json(graph): Json<OperationalDGraph>,
) -> Result<JsonResponse<OperationalDGraph>, ApiError> {
    ensure_operational_dgraph_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    validate_operational_dgraph(&graph).map_err(bad_request)?;
    if graph.project_id.trim() != project_id {
        return Err(bad_request(format!(
            "operational D-Graph project_id '{}' does not match X-Datum-Project '{}'",
            graph.project_id, project_id
        )));
    }

    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let current_graphs = graph_store().read().map_err(lock_error)?.clone();

    if let Some(existing) = current_graphs.get(&project_id) {
        if graph.revision < existing.revision {
            return Err(conflict(format!(
                "operational D-Graph revision regression: active revision is {}, received {}",
                existing.revision, graph.revision
            )));
        }
        if graph.revision == existing.revision {
            if &graph == existing {
                return Ok(JsonResponse(existing.clone()));
            }
            return Err(conflict(format!(
                "operational D-Graph revision {} is already active with different content; publish a higher revision",
                graph.revision
            )));
        }
    }

    let replacing_existing = current_graphs.contains_key(&project_id);
    let mut candidate = OperationalDGraphPersistedState {
        persistence_version: OPERATIONAL_DGRAPH_PERSISTENCE_VERSION.to_string(),
        graphs: current_graphs,
    };
    candidate.graphs.insert(project_id.clone(), graph.clone());

    persist_candidate(&candidate).map_err(internal_error)?;
    replace_in_memory_graphs(&candidate).map_err(internal_error)?;

    if replacing_existing {
        let prefix = format!("{}|", project_id);
        authorization_store()
            .write()
            .map_err(lock_error)?
            .retain(|key, _| !key.starts_with(&prefix));
    }

    Ok(JsonResponse(graph))
}

pub async fn get_operational_dgraph(
    headers: HeaderMap,
) -> Result<JsonResponse<OperationalDGraph>, ApiError> {
    ensure_operational_dgraph_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let graph = graph_store()
        .read()
        .map_err(lock_error)?
        .get(&project_id)
        .cloned()
        .ok_or_else(|| not_found(format!(
            "operational D-Graph not found for project '{}'",
            project_id
        )))?;
    Ok(JsonResponse(graph))
}

pub async fn get_operational_dgraph_slice(
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<OperationalDGraphSlice>, ApiError> {
    ensure_operational_dgraph_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let graph = graph_store()
        .read()
        .map_err(lock_error)?
        .get(&project_id)
        .cloned()
        .ok_or_else(|| not_found(format!(
            "operational D-Graph not found for project '{}'",
            project_id
        )))?;

    let authorization = authorization_store()
        .read()
        .map_err(lock_error)?
        .get(&authorization_key(&project_id, node_id.trim()))
        .cloned();

    let slice = build_operational_slice(&graph, node_id.trim(), authorization.as_ref())
        .map_err(not_found)?;
    Ok(JsonResponse(slice))
}

pub async fn post_operational_reconciliation_authorization(
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<OperationalReconciliationAuthorizationRequest>,
) -> Result<JsonResponse<OperationalReconciliationAuthorization>, ApiError> {
    ensure_operational_dgraph_persistence_initialized();
    let project_id = extract_project_id(&headers)?;
    let node_id = node_id.trim();

    let resource_bound_to_node = resource_registry_snapshot(&project_id)
        .map_err(internal_error)?
        .into_iter()
        .any(|resource| {
            resource.state == "bound"
                && resource.assigned_node_id.as_deref() == Some(node_id)
        });
    if !resource_bound_to_node {
        return Err(conflict(format!(
            "cannot authorize operational reconciliation for node '{}': no active Resource Registry binding exists",
            node_id
        )));
    }

    let graph = graph_store()
        .read()
        .map_err(lock_error)?
        .get(&project_id)
        .cloned()
        .ok_or_else(|| not_found(format!(
            "operational D-Graph not found for project '{}'",
            project_id
        )))?;

    let authorization = build_reconciliation_authorization(
        &graph,
        node_id,
        &request,
        Utc::now(),
    )
    .map_err(bad_request)?;

    authorization_store()
        .write()
        .map_err(lock_error)?
        .insert(
            authorization_key(&project_id, node_id),
            authorization.clone(),
        );

    Ok(JsonResponse(authorization))
}

fn persist_candidate(state: &OperationalDGraphPersistedState) -> Result<(), String> {
    let path = persistence_path_store()
        .read()
        .map_err(|error| format!("operational D-Graph persistence path lock poisoned: {error}"))?
        .clone()
        .ok_or_else(|| "operational D-Graph persistence path is not initialized".to_string())?;
    persist_operational_dgraph_state(&path, state)
}

fn replace_in_memory_graphs(state: &OperationalDGraphPersistedState) -> Result<(), String> {
    let mut graphs = graph_store()
        .write()
        .map_err(|error| format!("operational D-Graph state lock poisoned: {error}"))?;
    *graphs = state.graphs.clone();
    Ok(())
}

fn clear_in_memory_authorizations() -> Result<(), String> {
    authorization_store()
        .write()
        .map_err(|error| format!("operational authorization state lock poisoned: {error}"))?
        .clear();
    Ok(())
}

fn authorization_key(project_id: &str, node_id: &str) -> String {
    format!("{}|{}", project_id.trim(), node_id.trim())
}

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| bad_request("missing or invalid X-Datum-Project header".to_string()))
}

fn bad_request(message: String) -> ApiError {
    (StatusCode::BAD_REQUEST, JsonResponse(json!({"error": message})))
}

fn conflict(message: String) -> ApiError {
    (StatusCode::CONFLICT, JsonResponse(json!({"error": message})))
}

fn not_found(message: String) -> ApiError {
    (StatusCode::NOT_FOUND, JsonResponse(json!({"error": message})))
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
            "error": format!("operational D-Graph state lock poisoned: {error}")
        })),
    )
}
