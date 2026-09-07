use axum::{
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use serde_json::{json, Value};

use crate::{
    api::operational_dgraph::{
        operational_dgraph_slice_snapshot, operational_dgraph_snapshot,
    },
    core::operational_dgraph_acquisition::{
        build_active_operational_dgraph_acquisition, ActiveOperationalDGraphAcquisition,
    },
};

type ApiError = (StatusCode, Json<Value>);

pub async fn acquire_active_operational_dgraph(
    headers: HeaderMap,
) -> Result<Json<ActiveOperationalDGraphAcquisition>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let graph = operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| {
            not_found(format!(
                "operational D-Graph not found for project '{}'",
                project_id
            ))
        })?;

    let mut slices = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let slice = operational_dgraph_slice_snapshot(&project_id, &node.node_id)
            .map_err(internal_error)?
            .ok_or_else(|| {
                internal_error(format!(
                    "failed to materialize compatible operational D-Graph slice for node '{}'",
                    node.node_id
                ))
            })?;
        slices.push(slice);
    }

    let acquisition = build_active_operational_dgraph_acquisition(graph, slices, Utc::now())
        .map_err(internal_error)?;
    Ok(Json(acquisition))
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
    (StatusCode::BAD_REQUEST, Json(json!({"error": message})))
}

fn not_found(message: String) -> ApiError {
    (StatusCode::NOT_FOUND, Json(json!({"error": message})))
}

fn internal_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": message})),
    )
}
