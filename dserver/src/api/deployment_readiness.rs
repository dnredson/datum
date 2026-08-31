use axum::{extract::State, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde_json::{json, Value};

use crate::{
    api::{
        controlled_wasm_deployment::{
            controlled_live_reports_snapshot, controlled_manifest_snapshot,
            controlled_source_dgraph_snapshot,
        },
        deployment_lifecycle::attempts_snapshot,
        sentinel_registration::sentinel_registration_snapshot,
    },
    core::{
        controlled_dgraph_deployment::materialize_controlled_observed_dgraph,
        deployment_lifecycle_observed::materialize_lifecycle_evidence,
        deployment_readiness::{build_deployment_readiness, DeploymentReadinessSummary},
        dgraph::validate_dgraph,
        resource_observation::materialize_resource_observations,
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

pub async fn get_controlled_wasm_deployment_readiness(
    State(state): State<AppState>,
) -> Result<JsonResponse<DeploymentReadinessSummary>, ApiError> {
    let manifest = controlled_manifest_snapshot()
        .map_err(internal_error)?
        .ok_or_else(|| precondition("no active controlled WASM deployment manifest"))?;
    let source = controlled_source_dgraph_snapshot()
        .map_err(internal_error)?
        .ok_or_else(|| precondition("no external controlled D-Graph is active"))?;
    let reports = controlled_live_reports_snapshot().map_err(internal_error)?;

    let mut observed = materialize_controlled_observed_dgraph(&source, &manifest, &reports)
        .map_err(bad_request)?;
    let attempts = attempts_snapshot().map_err(internal_error)?;
    materialize_lifecycle_evidence(&mut observed, &manifest, &attempts).map_err(bad_request)?;

    let snapshots = state
        .firebase
        .list_latest_local_dgraph_snapshots(&manifest.project_id)
        .await
        .map_err(|error| internal_error(error.to_string()))?;
    materialize_resource_observations(&mut observed, &snapshots, Utc::now())
        .map_err(bad_request)?;
    observed.validation_errors = validate_dgraph(&observed.graph).err().unwrap_or_default();

    let registrations = sentinel_registration_snapshot().map_err(internal_error)?;
    let summary = build_deployment_readiness(
        &manifest,
        &observed,
        &registrations,
        &reports,
        Utc::now(),
    );
    Ok(JsonResponse(summary))
}

fn precondition(message: &str) -> ApiError {
    (
        StatusCode::PRECONDITION_FAILED,
        JsonResponse(json!({"error": message})),
    )
}

fn bad_request(message: String) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({"error": message})),
    )
}

fn internal_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message})),
    )
}
