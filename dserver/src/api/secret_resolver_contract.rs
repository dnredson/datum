use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    api::operational_dgraph::operational_dgraph_slice_snapshot,
    core::{
        artifact_reconciliation_context::{
            build_artifact_reconciliation_context, validate_artifact_reconciliation_context,
        },
        secret_resolver_contract::{
            build_secret_resolver_contract, SecretResolverContract,
        },
    },
    AppState,
};

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or((
            StatusCode::BAD_REQUEST,
            "missing or invalid X-Datum-Project header".to_string(),
        ))
}

pub async fn get_secret_resolver_contract(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SecretResolverContract>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let slice = operational_dgraph_slice_snapshot(&project_id, &node_id)
        .map_err(|error: String| (StatusCode::INTERNAL_SERVER_ERROR, error))?
        .ok_or((
            StatusCode::NOT_FOUND,
            format!(
                "operational D-Graph node slice not found for project '{project_id}' and node '{node_id}'"
            ),
        ))?;
    let artifacts = state
        .service_artifacts
        .list(&project_id)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let context = build_artifact_reconciliation_context(&slice, &artifacts.artifacts)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    validate_artifact_reconciliation_context(&context)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    build_secret_resolver_contract(&context)
        .map(Json)
        .map_err(|error| (StatusCode::BAD_REQUEST, error))
}
