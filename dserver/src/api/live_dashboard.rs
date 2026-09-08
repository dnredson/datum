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
    let graph_overview = build_graph_overview(&observed_dgraph);

    Ok(Json(json!({
        "schema_version": "0.1.0",
        "dashboard_kind": "datum_live_observability",
        "project_id": project_id,
        "generated_at_utc": Utc::now(),
        "observed_dgraph": observed_dgraph,
        "graph_overview": graph_overview,
        "telemetry": telemetry,
        "side_effects_performed": false
    })))
}

pub async fn get_graph_overview(
    headers: HeaderMap,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let Json(live_state) = get_live_dashboard_state(headers, Query(query)).await?;
    Ok(Json(
        live_state
            .get("graph_overview")
            .cloned()
            .unwrap_or_else(|| json!({
                "schema_version": "0.1.0",
                "graph_kind": "datum_observed_topology_graph",
                "nodes": [],
                "edges": [],
                "summary": {
                    "node_count": 0,
                    "edge_count": 0
                },
                "side_effects_performed": false
            })),
    ))
}

pub async fn get_live_dashboard_html() -> Html<&'static str> {
    Html(include_str!("../../static/live-dashboard.html"))
}

fn build_graph_overview(observed_dgraph: &Value) -> Value {
    let nodes = observed_dgraph
        .get("nodes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|node| {
                    let observed = node.get("observed");
                    let failure_state = node.get("failure_state");
                    json!({
                        "node_id": node.get("node_id").cloned().unwrap_or(Value::Null),
                        "stage": node
                            .get("node_id")
                            .and_then(Value::as_str)
                            .map(node_stage)
                            .unwrap_or("unclassified"),
                        "health_state": failure_state
                            .and_then(|value| value.get("state"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                        "failure_state": failure_state.cloned().unwrap_or(Value::Null),
                        "status": observed
                            .and_then(|value| value.get("status"))
                            .and_then(Value::as_str)
                            .unwrap_or("missing"),
                        "telemetry_freshness": observed
                            .and_then(|value| value.get("telemetry_freshness"))
                            .and_then(Value::as_str)
                            .unwrap_or("missing"),
                        "telemetry_age_seconds": observed
                            .and_then(|value| value.get("telemetry_age_seconds"))
                            .cloned()
                            .unwrap_or(Value::Null),
                        "sequence": observed
                            .and_then(|value| value.get("sequence"))
                            .cloned()
                            .unwrap_or(Value::Null),
                        "desired_service_count": node.get("desired_services").and_then(Value::as_array).map_or(0, Vec::len)
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let edges = observed_dgraph
        .get("measured_peer_edges")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, edge)| {
                    let reachable = edge.get("reachable").and_then(Value::as_bool);
                    let freshness = edge
                        .get("telemetry_freshness")
                        .and_then(Value::as_str)
                        .unwrap_or("missing");
                    json!({
                        "edge_id": format!(
                            "{}:{}:{}",
                            edge.get("from").and_then(Value::as_str).unwrap_or("unknown"),
                            edge.get("to").and_then(Value::as_str).unwrap_or("unknown"),
                            index
                        ),
                        "from": edge.get("from").cloned().unwrap_or(Value::Null),
                        "to": edge.get("to").cloned().unwrap_or(Value::Null),
                        "relation": edge.get("relation").cloned().unwrap_or_else(|| json!("peer_connectivity")),
                        "edge_status": peer_edge_status(reachable, freshness),
                        "reachable": edge.get("reachable").cloned().unwrap_or(Value::Null),
                        "rtt_ms": edge.get("rtt_ms").cloned().unwrap_or(Value::Null),
                        "consecutive_failures": edge.get("consecutive_failures").cloned().unwrap_or(Value::Null),
                        "telemetry_freshness": edge.get("telemetry_freshness").cloned().unwrap_or_else(|| json!("missing")),
                        "observed_at_utc": edge.get("observed_at_utc").cloned().unwrap_or(Value::Null),
                        "last_success_at_utc": edge.get("last_success_at_utc").cloned().unwrap_or(Value::Null),
                        "error": edge.get("error").cloned().unwrap_or(Value::Null)
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let count_status = |status: &str| {
        edges
            .iter()
            .filter(|edge| edge.get("edge_status").and_then(Value::as_str) == Some(status))
            .count()
    };

    json!({
        "schema_version": "0.1.0",
        "graph_kind": "datum_observed_topology_graph",
        "project_id": observed_dgraph.get("project_id").cloned().unwrap_or(Value::Null),
        "dgraph_id": observed_dgraph.get("dgraph_id").cloned().unwrap_or(Value::Null),
        "revision": observed_dgraph.get("revision").cloned().unwrap_or(Value::Null),
        "generated_at_utc": Utc::now(),
        "path_status": observed_dgraph.get("path_status").cloned().unwrap_or(Value::Null),
        "nodes": nodes,
        "edges": edges,
        "summary": {
            "node_count": nodes.len(),
            "edge_count": edges.len(),
            "up_edge_count": count_status("up"),
            "stale_edge_count": count_status("stale"),
            "degraded_edge_count": count_status("degraded"),
            "down_edge_count": count_status("down"),
            "unknown_edge_count": count_status("unknown")
        },
        "side_effects_performed": false
    })
}

fn node_stage(node_id: &str) -> &'static str {
    let normalized = node_id.trim().to_ascii_lowercase();
    if normalized.starts_with("device-") || normalized.starts_with("sensor-") {
        "device"
    } else if normalized.starts_with("edge-") {
        "edge"
    } else if normalized.starts_with("fog-") {
        "fog"
    } else if normalized.starts_with("cloud-") {
        "cloud"
    } else if normalized.starts_with("farm-") {
        "farm"
    } else {
        "unclassified"
    }
}

fn peer_edge_status(reachable: Option<bool>, freshness: &str) -> &'static str {
    match (reachable, freshness) {
        (Some(true), "fresh") => "up",
        (Some(true), _) => "stale",
        (Some(false), "fresh") => "down",
        (Some(false), _) => "degraded",
        _ => "unknown",
    }
}

fn internal_error(message: String) -> (StatusCode, Json<Value>) {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": message})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_overview_normalizes_nodes_and_peer_edge_status() {
        let observed = json!({
            "project_id": "monitor-f49c8",
            "dgraph_id": "dgraph:test",
            "revision": 5,
            "path_status": "all_expected_nodes_fresh",
            "nodes": [
                {
                    "node_id": "fog-01",
                    "desired_services": [{"service_id": "mqtt"}],
                    "observed": {
                        "status": "fresh",
                        "telemetry_freshness": "fresh",
                        "telemetry_age_seconds": 4,
                        "sequence": 2
                    }
                },
                {
                    "node_id": "cloud-01",
                    "desired_services": [],
                    "observed": {
                        "status": "fresh",
                        "telemetry_freshness": "fresh",
                        "telemetry_age_seconds": 3,
                        "sequence": 7
                    }
                }
            ],
            "measured_peer_edges": [
                {
                    "from": "fog-01",
                    "to": "cloud-01",
                    "relation": "peer_connectivity",
                    "reachable": true,
                    "rtt_ms": 5.2,
                    "telemetry_freshness": "fresh",
                    "consecutive_failures": 0
                }
            ]
        });

        let overview = build_graph_overview(&observed);

        assert_eq!(overview["graph_kind"], "datum_observed_topology_graph");
        assert_eq!(overview["summary"]["node_count"], 2);
        assert_eq!(overview["summary"]["edge_count"], 1);
        assert_eq!(overview["summary"]["up_edge_count"], 1);
        assert_eq!(overview["nodes"][0]["desired_service_count"], 1);
        assert_eq!(overview["nodes"][0]["stage"], "fog");
        assert_eq!(overview["edges"][0]["edge_status"], "up");
        assert_eq!(overview["edges"][0]["from"], "fog-01");
        assert_eq!(overview["edges"][0]["to"], "cloud-01");
        assert_eq!(overview["side_effects_performed"], false);
    }

    #[test]
    fn peer_edge_status_distinguishes_stale_and_unreachable_evidence() {
        assert_eq!(peer_edge_status(Some(true), "stale"), "stale");
        assert_eq!(peer_edge_status(Some(false), "fresh"), "down");
        assert_eq!(peer_edge_status(Some(false), "stale"), "degraded");
        assert_eq!(peer_edge_status(None, "missing"), "unknown");
    }
}
