use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    core::dgraph_readiness::{
        build_project_dgraph_readiness_summary, ProjectDGraphReadinessSummaryResponse,
    },
    AppState,
};

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|project_id| !project_id.is_empty())
        .map(str::to_string)
        .ok_or((
            StatusCode::BAD_REQUEST,
            "missing or invalid X-Datum-Project header".to_string(),
        ))
}

pub async fn get_dgraph_readiness_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProjectDGraphReadinessSummaryResponse>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let snapshots = state
        .firebase
        .list_latest_local_dgraph_snapshots(&project_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list_latest_local_dgraph_snapshots failed: {error}"),
            )
        })?;

    Ok(Json(build_project_dgraph_readiness_summary(
        &project_id,
        &snapshots,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_header_extraction_requires_x_datum_project() {
        let error = extract_project_id(&HeaderMap::new()).expect_err("header should be required");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }
}
