use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    api::operational_dgraph::operational_dgraph_slice_snapshot,
    core::artifact_reconciliation_context::{
        build_artifact_reconciliation_context, build_context_from_assignments,
        validate_artifact_reconciliation_context,
        ArtifactReconciliationContext,
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
    // P0-1: when a valid Stage 107A authorization carrying an immutable
    // snapshot is attached, the context MUST be built from that snapshot,
    // never by re-resolving the current (possibly since-mutated) artifact
    // registry -- that re-resolution is exactly what made Option A's "one
    // exact immutable execution snapshot" claim false in practice: an
    // artifact update between authorization and fetch would silently swap
    // in different content under the same still-valid authorization. A
    // missing authorization, or one issued before Stage 107A (empty
    // `snapshot`), falls back to the mutable-registry resolution -- this is
    // the unauthenticated/proposal-preview path, not a governed mutation.
    let context = match slice.authorization.as_ref().filter(|authorization| !authorization.snapshot.is_empty()) {
        Some(authorization) => build_context_from_assignments(&slice, authorization.snapshot.clone())
            .map_err(bad_request)?,
        None => {
            let artifacts = state.service_artifacts.list(&project_id)
                .map_err(|error| internal_error(error.to_string()))?;
            build_artifact_reconciliation_context(&slice, &artifacts.artifacts)
                .map_err(bad_request)?
        }
    };
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

