use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    core::wasm_runtime_reports::{
        normalize_wasm_runtime_report, validate_wasm_runtime_report, WasmRuntimeReport,
        WasmRuntimeReportList,
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

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

pub async fn post_wasm_runtime_report(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<WasmRuntimeReport>,
) -> Result<JsonResponse<WasmRuntimeReport>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let normalized = normalize_wasm_runtime_report(&project_id, &node_id, payload);

    validate_wasm_runtime_report(&normalized).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    state
        .firebase
        .put_wasm_runtime_report(&project_id, &node_id, &normalized)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(normalized))
}

pub async fn get_latest_wasm_runtime_report(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<WasmRuntimeReport>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let report = state
        .firebase
        .get_latest_wasm_runtime_report(&project_id, &node_id)
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
                    "wasm runtime report not found for project '{}' node '{}'",
                    project_id, node_id
                )
            })),
        )),
    }
}

pub async fn list_latest_wasm_runtime_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<WasmRuntimeReportList>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let reports = state
        .firebase
        .list_latest_wasm_runtime_reports(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(WasmRuntimeReportList {
        project_id,
        node_id: None,
        count: reports.len() as u64,
        reports,
    }))
}
