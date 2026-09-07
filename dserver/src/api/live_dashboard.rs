use axum::{
    extract::Query,
    http::{HeaderMap, StatusCode},
    response::Html,
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::{
        operational_dgraph::operational_dgraph_snapshot,
        sentinel_telemetry::build_observed_dgraph,
    },
    core::sentinel_telemetry::latest_telemetry_by_project,
};

#[derive(Debug, Deserialize, Default)]
pub struct DashboardQuery {
    pub project: Option<String>,
}

fn project_id(headers: &HeaderMap, query: &DashboardQuery) -> Result<String, (StatusCode, Json<Value>)> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| query.project.as_deref().map(str::trim).filter(|value| !value.is_empty()))
        .map(ToString::to_string)
        .ok_or_else(|| (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "missing X-Datum-Project header or project query parameter"})),
        ))
}

pub async fn get_live_dashboard_state(
    headers: HeaderMap,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let project_id = project_id(&headers, &query)?;
    let graph = operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("operational D-Graph not found for project '{project_id}'")})),
        ))?;
    let telemetry = latest_telemetry_by_project(&project_id, Utc::now())
        .map_err(internal_error)?;
    let observed_dgraph = build_observed_dgraph(&graph, telemetry.clone());

    Ok(Json(json!({
        "schema_version": "0.1.0",
        "dashboard_kind": "datum_live_observability",
        "project_id": project_id,
        "generated_at_utc": Utc::now(),
        "observed_dgraph": observed_dgraph,
        "telemetry": telemetry,
        "side_effects_performed": false
    })))
}

pub async fn get_live_dashboard_html() -> Html<&'static str> {
    Html(include_str!("../../static/live-dashboard.html"))
}

fn internal_error(message: String) -> (StatusCode, Json<Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": message})))
}
