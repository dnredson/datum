use axum::{
    extract::{Query, Json},
    http::{HeaderMap, StatusCode},
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;

use crate::{
    api::{
        operational_dgraph::operational_dgraph_snapshot,
        sentinel_telemetry::build_observed_dgraph,
    },
    core::{
        alternative_path_planner::build_alternative_path_what_if_context,
        external_ai_inference::{
            build_external_ai_inference_request,
            build_external_ai_inference_request_with_id,
            validate_external_ai_inference_response,
        },
        sentinel_telemetry::{
            telemetry_history_by_project, DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        },
    },
};

#[derive(Debug, Deserialize, Default)]
pub struct ExternalAiInferenceQuery {
    pub project: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub max_hops: Option<usize>,
}

pub async fn get_external_ai_inference_request(
    headers: HeaderMap,
    Query(query): Query<ExternalAiInferenceQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let context = load_context(&headers, &query).map_err(|error| error)?;
    let request = build_external_ai_inference_request(&context).map_err(bad_request)?;
    Ok(Json(request))
}

pub async fn post_external_ai_inference_response(
    headers: HeaderMap,
    Query(query): Query<ExternalAiInferenceQuery>,
    Json(response): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let context = load_context(&headers, &query).map_err(|error| error)?;
    let request_id = response
        .get("request_id")
        .and_then(Value::as_str);
    let request = build_external_ai_inference_request_with_id(&context, request_id)
        .map_err(bad_request)?;
    let validated = validate_external_ai_inference_response(&request, &response)
        .map_err(bad_request)?;
    Ok(Json(validated))
}

fn load_context(
    headers: &HeaderMap,
    query: &ExternalAiInferenceQuery,
) -> Result<Value, (StatusCode, Json<Value>)> {
    let project_id = extract_project_id(headers, query)?;
    let graph = operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": format!(
                        "operational D-Graph not found for project '{}'",
                        project_id
                    )
                })),
            )
        })?;

    let telemetry = crate::core::sentinel_telemetry::latest_telemetry_by_project(
        &project_id,
        Utc::now(),
    )
    .map_err(internal_error)?;
    let observed_dgraph = build_observed_dgraph(&graph, telemetry);
    let history = telemetry_history_by_project(
        &project_id,
        DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        Utc::now(),
    )
    .map_err(internal_error)?;

    build_alternative_path_what_if_context(
        &observed_dgraph,
        history,
        query.from.as_deref(),
        query.to.as_deref(),
        query.max_hops,
    )
    .map_err(bad_request)
}

fn extract_project_id(
    headers: &HeaderMap,
    query: &ExternalAiInferenceQuery,
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
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "missing X-Datum-Project header or project query parameter"
                })),
            )
        })
}

fn bad_request(message: String) -> (StatusCode, Json<Value>) {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": message })))
}

fn internal_error(message: String) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": message })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_has_priority_over_query_project() {
        let mut headers = HeaderMap::new();
        headers.insert("x-datum-project", "header-project".parse().unwrap());
        let query = ExternalAiInferenceQuery {
            project: Some("query-project".to_string()),
            ..Default::default()
        };
        assert_eq!(
            extract_project_id(&headers, &query).unwrap(),
            "header-project"
        );
    }

    #[test]
    fn query_project_is_supported_without_header() {
        let query = ExternalAiInferenceQuery {
            project: Some("monitor-f49c8".to_string()),
            ..Default::default()
        };
        assert_eq!(
            extract_project_id(&HeaderMap::new(), &query).unwrap(),
            "monitor-f49c8"
        );
    }
}
