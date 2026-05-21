use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    core::wasm_module_registry::{
        normalize_wasm_module_descriptor, validate_wasm_module_descriptor, WasmModuleDescriptor,
        WasmModuleDescriptorList,
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

pub async fn post_wasm_module_descriptor(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WasmModuleDescriptor>,
) -> Result<JsonResponse<WasmModuleDescriptor>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let normalized = normalize_wasm_module_descriptor(&project_id, payload);

    validate_wasm_module_descriptor(&normalized).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    state
        .firebase
        .put_wasm_module_descriptor(&project_id, &normalized.module_id, &normalized)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(normalized))
}

pub async fn list_wasm_module_descriptors(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<WasmModuleDescriptorList>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let modules = state
        .firebase
        .list_wasm_module_descriptors(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(WasmModuleDescriptorList {
        project_id,
        count: modules.len() as u64,
        modules,
    }))
}

pub async fn get_wasm_module_descriptor(
    State(state): State<AppState>,
    Path(module_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<WasmModuleDescriptor>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let module = state
        .firebase
        .get_wasm_module_descriptor(&project_id, &module_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match module {
        Some(module) => Ok(JsonResponse(module)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "wasm module descriptor not found for project '{}' module '{}'",
                    project_id, module_id
                )
            })),
        )),
    }
}
