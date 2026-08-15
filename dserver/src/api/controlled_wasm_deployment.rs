use std::sync::{OnceLock, RwLock};

use axum::{
    extract::{Json, Path},
    http::StatusCode,
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::core::{
    controlled_wasm_deployment::{
        build_controlled_live_report_list, report_execution_id,
        validate_controlled_live_report_against_manifest, ControlledWasmLiveReportList,
    },
    deployment_assignment::{derive_node_scoped_deployment, NodeScopedDeployment},
    deployment_manifest::{validate_deployment_manifest, DeploymentManifest},
};

type ApiError = (StatusCode, JsonResponse<Value>);

static ACTIVE_MANIFEST: OnceLock<RwLock<Option<DeploymentManifest>>> = OnceLock::new();
static LIVE_REPORTS: OnceLock<RwLock<Vec<Value>>> = OnceLock::new();

fn manifest_store() -> &'static RwLock<Option<DeploymentManifest>> {
    ACTIVE_MANIFEST.get_or_init(|| RwLock::new(None))
}

fn report_store() -> &'static RwLock<Vec<Value>> {
    LIVE_REPORTS.get_or_init(|| RwLock::new(Vec::new()))
}

pub async fn post_controlled_wasm_deployment_manifest(
    Json(payload): Json<DeploymentManifest>,
) -> Result<JsonResponse<DeploymentManifest>, ApiError> {
    validate_deployment_manifest(&payload).map_err(bad_request)?;

    {
        let mut active = manifest_store().write().map_err(lock_error)?;
        *active = Some(payload.clone());
    }
    {
        let mut reports = report_store().write().map_err(lock_error)?;
        reports.clear();
    }

    Ok(JsonResponse(payload))
}

pub async fn get_controlled_wasm_deployment_manifest(
) -> Result<JsonResponse<DeploymentManifest>, ApiError> {
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
    let manifest = {
        let active = manifest_store().read().map_err(lock_error)?;
        active.clone().ok_or_else(|| {
            (
                StatusCode::PRECONDITION_FAILED,
                JsonResponse(json!({
                    "error": "publish the controlled WASM deployment manifest before live reports"
                })),
            )
        })?
    };

    validate_controlled_live_report_against_manifest(&manifest, &payload).map_err(bad_request)?;
    let execution_id = report_execution_id(&payload)
        .expect("validated report always has execution_id")
        .to_string();

    let mut reports = report_store().write().map_err(lock_error)?;
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

    Ok(JsonResponse(payload))
}

pub async fn list_controlled_wasm_live_reports(
) -> Result<JsonResponse<ControlledWasmLiveReportList>, ApiError> {
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
    let reports = report_store().read().map_err(lock_error)?;
    Ok(JsonResponse(build_controlled_live_report_list(
        &manifest,
        &reports,
    )))
}

fn bad_request(message: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({ "error": message })),
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
