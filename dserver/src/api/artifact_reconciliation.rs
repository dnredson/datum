use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    core::{
        artifact_reconciliation_context::{
            build_artifact_reconciliation_context,
            validate_artifact_reconciliation_context,
            ArtifactReconciliationContext,
        },
        operational_dgraph::operational_dgraph_slice_snapshot,
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({"error":"missing or invalid X-Datum-Project header"})),
        ))
}

pub async fn get_operational_reconciliation_context(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ArtifactReconciliationContext>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let slice = operational_dgraph_slice_snapshot(&project_id, &node_id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found(format!(
            "operational D-Graph node slice not found for project '{}' and node '{}'",
            project_id, node_id
        )))?;
    let artifacts = state.service_artifacts.list(&project_id)
        .map_err(|error| internal_error(error.to_string()))?;
    let context = build_artifact_reconciliation_context(&slice, &artifacts.artifacts)
        .map_err(bad_request)?;
    validate_artifact_reconciliation_context(&context).map_err(bad_request)?;
    Ok(JsonResponse(context))
}

fn bad_request(message: String) -> ApiError {
    (StatusCode::BAD_REQUEST, JsonResponse(json!({"error":message})))
}
fn not_found(message: String) -> ApiError {
    (StatusCode::NOT_FOUND, JsonResponse(json!({"error":message})))
}
fn internal_error(message: String) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error":message})))
}

