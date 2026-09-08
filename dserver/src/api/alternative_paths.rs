use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    api::{
        operational_dgraph::operational_dgraph_snapshot,
        sentinel_telemetry::build_observed_dgraph,
    },
    core::alternative_path_planner::plan_alternative_paths,
};

#[derive(Debug, Deserialize, Default)]
pub struct AlternativePathQuery {
    pub project: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub max_hops: Option<usize>,
}

fn extract_project_id(
    headers: &HeaderMap,
    query: &AlternativePathQuery,
) -> Result<String, (StatusCode, Json<Value>)> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            query
                .project
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(ToString::to_string)
        .ok_or_else(|| (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing X-Datum-Project header or project query parameter"
            })),
        ))
}

pub async fn get_alternative_paths(
    headers: HeaderMap,
    Query(query): Query<AlternativePathQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project_id = extract_project_id(&headers, &query)?;
    let graph = operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!(
                    "operational D-Graph not found for project '{}'",
                    project_id
                )
            })),
        ))?;

    let telemetry = crate::core::sentinel_telemetry::latest_telemetry_by_project(
        &project_id,
        Utc::now(),
    )
    .map_err(internal_error)?;
    let observed_dgraph = build_observed_dgraph(&graph, telemetry);

    let response = plan_alternative_paths(
        &observed_dgraph,
        query.from.as_deref(),
        query.to.as_deref(),
        query.max_hops,
    )
    .map_err(|message| (StatusCode::BAD_REQUEST, Json(serde_json::json!({
        "error": message
    }))))?;

    Ok(Json(response))
}

fn internal_error(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": message})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_query_is_accepted_when_header_is_absent() {
        let headers = HeaderMap::new();
        let query = AlternativePathQuery {
            project: Some("monitor-f49c8".to_string()),
            ..Default::default()
        };

        assert_eq!(
            extract_project_id(&headers, &query).unwrap(),
            "monitor-f49c8"
        );
    }

    #[test]
    fn blank_project_is_rejected() {
        let headers = HeaderMap::new();
        let query = AlternativePathQuery {
            project: Some(" ".to_string()),
            ..Default::default()
        };

        let error = extract_project_id(&headers, &query).unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }
}
