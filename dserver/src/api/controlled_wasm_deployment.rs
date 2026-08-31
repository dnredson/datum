use std::{
    env,
    path::{Path as FsPath, PathBuf},
    sync::{Mutex, OnceLock, RwLock},
};

use axum::{
    extract::{Json, Path},
    http::StatusCode,
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::core::{
    controlled_dgraph_deployment::{
        build_controlled_dgraph_activation, materialize_controlled_observed_dgraph,
        ControlledDGraphActivation, ControlledObservedDGraphState,
    },
    controlled_wasm_deployment::{
        build_controlled_live_report_list, report_execution_id,
        validate_controlled_live_report_against_manifest, ControlledWasmLiveReportList,
    },
    controlled_wasm_persistence::{
        load_controlled_wasm_persisted_state, persist_controlled_wasm_state,
        ControlledWasmPersistedState, CONTROLLED_WASM_PERSISTENCE_VERSION,
    },
    deployment_assignment::{derive_node_scoped_deployment, NodeScopedDeployment},
    deployment_lifecycle_observed::materialize_lifecycle_evidence,
    deployment_manifest::{validate_deployment_manifest, DeploymentManifest},
    dgraph::DGraph,
};

type ApiError = (StatusCode, JsonResponse<Value>);

const DEFAULT_CONTROLLED_WASM_STATE_PATH: &str = ".datum-controlled-wasm-state.json";
const CONTROLLED_WASM_STATE_PATH_ENV: &str = "DATUM_CONTROLLED_WASM_STATE_PATH";

static ACTIVE_MANIFEST: OnceLock<RwLock<Option<DeploymentManifest>>> = OnceLock::new();
static SOURCE_DGRAPH: OnceLock<RwLock<Option<DGraph>>> = OnceLock::new();
static LIVE_REPORTS: OnceLock<RwLock<Vec<Value>>> = OnceLock::new();
static PERSISTENCE_PATH: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
static STATE_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();
static PERSISTENCE_INITIALIZED: OnceLock<()> = OnceLock::new();

fn manifest_store() -> &'static RwLock<Option<DeploymentManifest>> {
    ACTIVE_MANIFEST.get_or_init(|| RwLock::new(None))
}

fn source_dgraph_store() -> &'static RwLock<Option<DGraph>> {
    SOURCE_DGRAPH.get_or_init(|| RwLock::new(None))
}

fn report_store() -> &'static RwLock<Vec<Value>> {
    LIVE_REPORTS.get_or_init(|| RwLock::new(Vec::new()))
}

fn persistence_path_store() -> &'static RwLock<Option<PathBuf>> {
    PERSISTENCE_PATH.get_or_init(|| RwLock::new(None))
}

fn state_transaction() -> &'static Mutex<()> {
    STATE_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn ensure_controlled_wasm_persistence_initialized() {
    PERSISTENCE_INITIALIZED.get_or_init(|| {
        let path = env::var(CONTROLLED_WASM_STATE_PATH_ENV)
            .unwrap_or_else(|_| DEFAULT_CONTROLLED_WASM_STATE_PATH.to_string());
        match initialize_controlled_wasm_persistence(FsPath::new(&path)) {
            Ok(true) => {
                println!("restored controlled WASM deployment state from {path}");
            }
            Ok(false) => {}
            Err(error) => {
                eprintln!(
                    "controlled WASM persisted state was not activated from {path}: {error}; starting controlled state empty"
                );
            }
        }
    });
}

pub fn initialize_controlled_wasm_persistence(path: &FsPath) -> Result<bool, String> {
    let _transaction = state_transaction()
        .lock()
        .map_err(|error| format!("controlled WASM state transaction lock poisoned: {error}"))?;

    {
        let mut configured = persistence_path_store()
            .write()
            .map_err(|error| format!("controlled WASM persistence path lock poisoned: {error}"))?;
        *configured = Some(path.to_path_buf());
    }

    match load_controlled_wasm_persisted_state(path) {
        Ok(Some(state)) => {
            replace_in_memory_state_string(&state)?;
            Ok(true)
        }
        Ok(None) => {
            replace_in_memory_state_string(&ControlledWasmPersistedState::empty())?;
            Ok(false)
        }
        Err(error) => {
            replace_in_memory_state_string(&ControlledWasmPersistedState::empty())?;
            Err(error)
        }
    }
}

pub async fn post_controlled_wasm_dgraph(
    Json(payload): Json<DGraph>,
) -> Result<JsonResponse<ControlledDGraphActivation>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let activation = build_controlled_dgraph_activation(&payload).map_err(bad_request)?;
    let _transaction = state_transaction().lock().map_err(lock_error)?;

    let candidate = ControlledWasmPersistedState {
        persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
        source_dgraph: Some(payload),
        active_manifest: Some(activation.manifest.clone()),
        live_reports: Vec::new(),
    };
    persist_candidate(&candidate)?;
    replace_in_memory_state(&candidate)?;
    crate::api::deployment_lifecycle::clear_attempts().map_err(bad_request)?;

    Ok(JsonResponse(activation))
}

pub async fn get_controlled_wasm_dgraph() -> Result<JsonResponse<DGraph>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let source = source_dgraph_store().read().map_err(lock_error)?;
    source.clone().map(JsonResponse).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            JsonResponse(json!({"error": "no external controlled D-Graph has been published"})),
        )
    })
}

pub async fn get_controlled_wasm_observed_dgraph(
) -> Result<JsonResponse<ControlledObservedDGraphState>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let source = source_dgraph_store()
        .read()
        .map_err(lock_error)?
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({"error": "no external controlled D-Graph has been published"})),
            )
        })?;
    let manifest = manifest_store()
        .read()
        .map_err(lock_error)?
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::PRECONDITION_FAILED,
                JsonResponse(json!({"error": "no active manifest exists for the external D-Graph"})),
            )
        })?;
    let reports = report_store().read().map_err(lock_error)?.clone();
    let mut observed = materialize_controlled_observed_dgraph(&source, &manifest, &reports)
        .map_err(bad_request)?;
    let attempts = crate::api::deployment_lifecycle::attempts_snapshot().map_err(bad_request)?;
    materialize_lifecycle_evidence(&mut observed, &manifest, &attempts).map_err(bad_request)?;
    Ok(JsonResponse(observed))
}

pub async fn post_controlled_wasm_deployment_manifest(
    Json(payload): Json<DeploymentManifest>,
) -> Result<JsonResponse<DeploymentManifest>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    validate_deployment_manifest(&payload).map_err(bad_request)?;
    let _transaction = state_transaction().lock().map_err(lock_error)?;

    let candidate = ControlledWasmPersistedState {
        persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
        source_dgraph: None,
        active_manifest: Some(payload.clone()),
        live_reports: Vec::new(),
    };
    persist_candidate(&candidate)?;
    replace_in_memory_state(&candidate)?;
    crate::api::deployment_lifecycle::clear_attempts().map_err(bad_request)?;

    Ok(JsonResponse(payload))
}

pub async fn get_controlled_wasm_deployment_manifest(
) -> Result<JsonResponse<DeploymentManifest>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let active = manifest_store().read().map_err(lock_error)?;
    active.clone().map(JsonResponse).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": "no controlled WASM deployment manifest has been published"
            })),
        )
    })
}

pub async fn get_controlled_wasm_deployment_for_node(
    Path(node_id): Path<String>,
) -> Result<JsonResponse<NodeScopedDeployment>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let manifest = {
        let active = manifest_store().read().map_err(lock_error)?;
        active.clone().ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": "no controlled WASM deployment manifest has been published"
                })),
            )
        })?
    };

    let view = derive_node_scoped_deployment(&manifest, &node_id).map_err(bad_request)?;
    Ok(JsonResponse(view))
}

pub async fn post_controlled_wasm_live_report(
    Json(payload): Json<Value>,
) -> Result<JsonResponse<Value>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let _transaction = state_transaction().lock().map_err(lock_error)?;
    let manifest = manifest_store()
        .read()
        .map_err(lock_error)?
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::PRECONDITION_FAILED,
                JsonResponse(json!({
                    "error": "publish the controlled WASM deployment manifest before live reports"
                })),
            )
        })?;

    validate_controlled_live_report_against_manifest(&manifest, &payload).map_err(bad_request)?;
    let execution_id = report_execution_id(&payload)
        .expect("validated report always has execution_id")
        .to_string();

    let mut reports = report_store().read().map_err(lock_error)?.clone();
    if reports
        .iter()
        .any(|report| report_execution_id(report) == Some(execution_id.as_str()))
    {
        return Err((
            StatusCode::CONFLICT,
            JsonResponse(json!({
                "error": format!("live report execution_id '{}' already exists", execution_id)
            })),
        ));
    }
    reports.push(payload.clone());

    let source = source_dgraph_store().read().map_err(lock_error)?.clone();
    let candidate = ControlledWasmPersistedState {
        persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
        source_dgraph: source,
        active_manifest: Some(manifest),
        live_reports: reports,
    };
    persist_candidate(&candidate)?;
    replace_in_memory_state(&candidate)?;

    Ok(JsonResponse(payload))
}

pub async fn list_controlled_wasm_live_reports(
) -> Result<JsonResponse<ControlledWasmLiveReportList>, ApiError> {
    ensure_controlled_wasm_persistence_initialized();
    let manifest = manifest_store()
        .read()
        .map_err(lock_error)?
        .clone()
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({
                    "error": "no controlled WASM deployment manifest has been published"
                })),
            )
        })?;
    let reports = report_store().read().map_err(lock_error)?;
    Ok(JsonResponse(build_controlled_live_report_list(
        &manifest,
        &reports,
    )))
}

pub fn controlled_manifest_snapshot() -> Result<Option<DeploymentManifest>, String> {
    ensure_controlled_wasm_persistence_initialized();
    manifest_store()
        .read()
        .map(|guard| guard.clone())
        .map_err(|error| format!("controlled WASM manifest lock poisoned: {error}"))
}

pub fn controlled_source_dgraph_snapshot() -> Result<Option<DGraph>, String> {
    ensure_controlled_wasm_persistence_initialized();
    source_dgraph_store()
        .read()
        .map(|guard| guard.clone())
        .map_err(|error| format!("controlled WASM source D-Graph lock poisoned: {error}"))
}

pub fn controlled_live_reports_snapshot() -> Result<Vec<Value>, String> {
    ensure_controlled_wasm_persistence_initialized();
    report_store()
        .read()
        .map(|guard| guard.clone())
        .map_err(|error| format!("controlled WASM report lock poisoned: {error}"))
}

fn persist_candidate(state: &ControlledWasmPersistedState) -> Result<(), ApiError> {
    let path = persistence_path_store()
        .read()
        .map_err(lock_error)?
        .clone();
    if let Some(path) = path {
        persist_controlled_wasm_state(&path, state).map_err(persistence_error)?;
    }
    Ok(())
}

fn replace_in_memory_state(state: &ControlledWasmPersistedState) -> Result<(), ApiError> {
    {
        let mut source = source_dgraph_store().write().map_err(lock_error)?;
        *source = state.source_dgraph.clone();
    }
    {
        let mut manifest = manifest_store().write().map_err(lock_error)?;
        *manifest = state.active_manifest.clone();
    }
    {
        let mut reports = report_store().write().map_err(lock_error)?;
        *reports = state.live_reports.clone();
    }
    Ok(())
}

fn replace_in_memory_state_string(state: &ControlledWasmPersistedState) -> Result<(), String> {
    {
        let mut source = source_dgraph_store()
            .write()
            .map_err(|error| format!("controlled WASM source D-Graph lock poisoned: {error}"))?;
        *source = state.source_dgraph.clone();
    }
    {
        let mut manifest = manifest_store()
            .write()
            .map_err(|error| format!("controlled WASM manifest lock poisoned: {error}"))?;
        *manifest = state.active_manifest.clone();
    }
    {
        let mut reports = report_store()
            .write()
            .map_err(|error| format!("controlled WASM report lock poisoned: {error}"))?;
        *reports = state.live_reports.clone();
    }
    Ok(())
}

fn bad_request(message: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({ "error": message })),
    )
}

fn persistence_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({
            "error": format!("controlled WASM state persistence failed: {message}")
        })),
    )
}

fn lock_error<T>(error: std::sync::PoisonError<T>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({
            "error": format!("controlled WASM deployment state lock poisoned: {error}")
        })),
    )
}
