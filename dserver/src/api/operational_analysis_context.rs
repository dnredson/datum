use std::{collections::BTreeMap, env};

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use serde_json::{json, Value};

use crate::{
    api::operational_dgraph::operational_dgraph_snapshot,
    core::operational_analysis_context::{
        build_active_analysis_context, build_passive_analysis_context, OperationalAnalysisContext,
        DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS,
    },
    AppState,
};

type ApiError = (StatusCode, Json<Value>);

const TELEMETRY_FRESHNESS_MAX_AGE_SECONDS_ENV: &str =
    "DATUM_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS";

pub async fn acquire_passive_analysis_context(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<OperationalAnalysisContext>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let node_id = node_id.trim().to_string();
    if node_id.is_empty() {
        return Err(bad_request("node_id must not be empty".to_string()));
    }

    let context = acquire_passive_analysis_context_for_node(&state, &project_id, &node_id)
        .await
        .map_err(not_found)?;
    Ok(Json(context))
}

pub async fn acquire_passive_analysis_context_for_node(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<OperationalAnalysisContext, String> {
    let project_id = project_id.trim();
    let node_id = node_id.trim();
    if project_id.is_empty() {
        return Err("project_id must not be empty".to_string());
    }
    if node_id.is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    let graph = operational_dgraph_snapshot(project_id)?
        .ok_or_else(|| format!("operational D-Graph not found for project '{}'", project_id))?;
    let telemetry = state
        .firebase
        .get_latest_snapshot(project_id, node_id)
        .await
        .map_err(|error| error.to_string());
    let freshness_max_age_seconds = telemetry_freshness_max_age_seconds_string()?;

    build_passive_analysis_context(
        &graph,
        node_id,
        telemetry,
        Utc::now(),
        freshness_max_age_seconds,
    )
}

pub async fn acquire_active_analysis_context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<OperationalAnalysisContext>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let graph = operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| {
            not_found(format!(
                "operational D-Graph not found for project '{}'",
                project_id
            ))
        })?;

    // Spawn all Firebase reads before awaiting any of them so active graph scans
    // do not accumulate one network round-trip per node serially.
    let mut handles = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let firebase = state.firebase.clone();
        let project_id = project_id.clone();
        let node_id = node.node_id.clone();
        let handle = tokio::spawn(async move {
            firebase
                .get_latest_snapshot(&project_id, &node_id)
                .await
                .map_err(|error| error.to_string())
        });
        handles.push((node.node_id.clone(), handle));
    }

    let mut telemetry_by_node = BTreeMap::new();
    for (node_id, handle) in handles {
        let telemetry = match handle.await {
            Ok(result) => result,
            Err(error) => Err(format!(
                "Firebase telemetry acquisition task failed for node '{}': {}",
                node_id, error
            )),
        };
        telemetry_by_node.insert(node_id, telemetry);
    }

    let freshness_max_age_seconds = telemetry_freshness_max_age_seconds()?;
    let context = build_active_analysis_context(
        &graph,
        telemetry_by_node,
        Utc::now(),
        freshness_max_age_seconds,
    )
    .map_err(internal_error)?;
    Ok(Json(context))
}

fn telemetry_freshness_max_age_seconds_string() -> Result<u64, String> {
    match env::var(TELEMETRY_FRESHNESS_MAX_AGE_SECONDS_ENV) {
        Ok(value) if !value.trim().is_empty() => {
            let parsed = value.trim().parse::<u64>().map_err(|error| {
                format!(
                    "invalid {}='{}': expected positive integer seconds: {}",
                    TELEMETRY_FRESHNESS_MAX_AGE_SECONDS_ENV, value, error
                )
            })?;
            if parsed == 0 {
                return Err(format!(
                    "invalid {}='0': freshness max age must be greater than zero",
                    TELEMETRY_FRESHNESS_MAX_AGE_SECONDS_ENV
                ));
            }
            Ok(parsed)
        }
        _ => Ok(DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS),
    }
}

fn telemetry_freshness_max_age_seconds() -> Result<u64, ApiError> {
    telemetry_freshness_max_age_seconds_string().map_err(internal_error)
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
