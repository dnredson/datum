use std::{collections::BTreeMap, env, fmt};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::operational_dgraph::{
        operational_dgraph_acquisition_snapshot, operational_dgraph_snapshot,
    },
    core::{
        active_ai_context::build_active_ai_context,
        operational_analysis_context::{
            build_active_analysis_context, build_passive_analysis_context,
            OperationalAnalysisContext, DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS,
        },
        operational_dgraph::OperationalDGraph,
        operational_dgraph_acquisition::build_active_operational_dgraph_acquisition,
    },
    AppState,
};

type ApiError = (StatusCode, Json<Value>);

const TELEMETRY_FRESHNESS_MAX_AGE_SECONDS_ENV: &str =
    "DATUM_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS";

#[derive(Debug, Default, Deserialize)]
pub struct ActiveAnalysisQuery {
    pub format: Option<String>,
}

#[derive(Debug)]
pub enum PassiveAnalysisAcquireError {
    NotFound(String),
    Internal(String),
}

impl fmt::Display for PassiveAnalysisAcquireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(message) | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

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

    match acquire_passive_analysis_context_for_node(&state, &project_id, &node_id).await {
        Ok(context) => Ok(Json(context)),
        Err(PassiveAnalysisAcquireError::NotFound(message)) => Err(not_found(message)),
        Err(PassiveAnalysisAcquireError::Internal(message)) => Err(internal_error(message)),
    }
}

pub async fn acquire_passive_analysis_context_for_node(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<OperationalAnalysisContext, PassiveAnalysisAcquireError> {
    let project_id = project_id.trim();
    let node_id = node_id.trim();
    if project_id.is_empty() {
        return Err(PassiveAnalysisAcquireError::NotFound(
            "project_id must not be empty".to_string(),
        ));
    }
    if node_id.is_empty() {
        return Err(PassiveAnalysisAcquireError::NotFound(
            "node_id must not be empty".to_string(),
        ));
    }

    let graph = operational_dgraph_snapshot(project_id)
        .map_err(PassiveAnalysisAcquireError::Internal)?
        .ok_or_else(|| {
            PassiveAnalysisAcquireError::NotFound(format!(
                "operational D-Graph not found for project '{}'",
                project_id
            ))
        })?;
    let telemetry = state
        .firebase
        .get_latest_snapshot(project_id, node_id)
        .await
        .map_err(|error| error.to_string());
    let freshness_max_age_seconds = telemetry_freshness_max_age_seconds_string()
        .map_err(PassiveAnalysisAcquireError::Internal)?;

    build_passive_analysis_context(
        &graph,
        node_id,
        telemetry,
        Utc::now(),
        freshness_max_age_seconds,
    )
    .map_err(PassiveAnalysisAcquireError::NotFound)
}

/// Preserve the existing analysis and AI contracts. The complete D-Graph
/// format uses one captured accepted revision and never performs reconciliation.
pub async fn acquire_active_analysis_context(
    State(state): State<AppState>,
    Query(query): Query<ActiveAnalysisQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    match query.format.as_deref() {
        Some("dgraph") => {
            let (graph, slices) = operational_dgraph_acquisition_snapshot(&project_id)
                .map_err(internal_error)?
                .ok_or_else(|| {
                    not_found(format!(
                        "operational D-Graph not found for project '{}'",
                        project_id
                    ))
                })?;
            let package = build_active_operational_dgraph_acquisition(graph, slices, Utc::now())
                .map_err(internal_error)?;
            Ok(Json(
                serde_json::to_value(package)
                    .map_err(|error| internal_error(error.to_string()))?,
            ))
        }
        None | Some("analysis") | Some("ai") => {
            let ai_format = matches!(query.format.as_deref(), Some("ai"));
            let (graph, context) =
                acquire_active_analysis_context_for_project(&state, &project_id).await?;
            if ai_format {
                let package = build_active_ai_context(graph, context).map_err(internal_error)?;
                Ok(Json(
                    serde_json::to_value(package)
                        .map_err(|error| internal_error(error.to_string()))?,
                ))
            } else {
                Ok(Json(
                    serde_json::to_value(context)
                        .map_err(|error| internal_error(error.to_string()))?,
                ))
            }
        }
        Some(_) => Err(bad_request(
            "format must be 'analysis', 'ai', or 'dgraph'".to_string(),
        )),
    }
}

/// Read one accepted revision and acquire every desired node concurrently.
/// The assessment is derived from that same revision. Individual Firebase
/// failures remain visible rather than aborting the entire active scan.
async fn acquire_active_analysis_context_for_project(
    state: &AppState,
    project_id: &str,
) -> Result<(OperationalDGraph, OperationalAnalysisContext), ApiError> {
    let graph = operational_dgraph_snapshot(project_id)
        .map_err(internal_error)?
        .ok_or_else(|| {
            not_found(format!(
                "operational D-Graph not found for project '{}'",
                project_id
            ))
        })?;

    let mut handles = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let firebase = state.firebase.clone();
        let project_id = project_id.to_string();
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
    Ok((graph, context))
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
