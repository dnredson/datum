use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    core::service_artifact_registry::{
        ServiceArtifact, ServiceArtifactList, ServiceArtifactRegistryError, ServiceArtifactRegistry,
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    project_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "missing or invalid X-Datum-Project header" })),
        )
    })
}

fn map_registry_error(error: ServiceArtifactRegistryError) -> ApiError {
    let status = match error {
        ServiceArtifactRegistryError::Invalid(_) => StatusCode::BAD_REQUEST,
        ServiceArtifactRegistryError::Conflict(_) => StatusCode::CONFLICT,
        ServiceArtifactRegistryError::NotFound(_) => StatusCode::NOT_FOUND,
        ServiceArtifactRegistryError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, JsonResponse(json!({ "error": error.message() })))
}

pub async fn list_service_artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<ServiceArtifactList>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .list(&project_id)
        .map(JsonResponse)
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": error })),
            )
        })
}

pub async fn get_service_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .get(&project_id, &artifact_id)
        .map(JsonResponse)
        .map_err(map_registry_error)
}

pub async fn post_service_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ServiceArtifact>,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .create(&project_id, payload)
        .map(JsonResponse)
        .map_err(map_registry_error)
}

pub async fn put_service_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ServiceArtifact>,
) -> Result<JsonResponse<ServiceArtifact>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .update(&project_id, &artifact_id, payload)
        .map(JsonResponse)
        .map_err(map_registry_error)
}

pub async fn delete_service_artifact(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let project_id = extract_project_id(&headers)?;
    state
        .service_artifacts
        .delete(&project_id, &artifact_id)
        .map_err(map_registry_error)?;
    Ok(StatusCode::NO_CONTENT)
}
