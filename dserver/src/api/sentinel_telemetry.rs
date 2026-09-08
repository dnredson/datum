use axum::{
    extract::{Json, Path, Query},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    core::{
        operational_dgraph::OperationalDGraph,
        sentinel_telemetry::{
            ingest_telemetry, latest_telemetry, latest_telemetry_by_project,
            telemetry_history, telemetry_history_count, update_failure_state,
            TelemetryIngestDisposition, TelemetryObservationView,
            DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        },
    },
    api::operational_dgraph::operational_dgraph_snapshot,
};

type ApiError = (StatusCode, JsonResponse<Value>);

#[derive(Debug, Deserialize)]
pub struct TelemetryHistoryQuery {
    pub limit: Option<usize>,
}

pub async fn post_sentinel_telemetry(
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = project_id(&headers)?;
    let (observation, disposition, history_count) =
        ingest_telemetry(&project_id, &node_id, payload, Utc::now()).map_err(bad_request)?;

    Ok(JsonResponse(json!({
        "accepted": true,
        "duplicate": disposition == TelemetryIngestDisposition::Duplicate,
        "project_id": project_id,
        "node_id": observation.node_id,
        "sequence": observation.sequence,
        "observed_at_utc": observation.observed_at_utc,
        "received_at_utc": observation.received_at_utc,
        "telemetry_age_seconds": observation.telemetry_age_seconds,
        "telemetry_freshness": observation.telemetry_freshness,
        "history_count": history_count,
        "history_capacity": DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        "side_effects_performed": false
    })))
}

pub async fn get_sentinel_telemetry(
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = project_id(&headers)?;
    let observation = latest_telemetry(&project_id, &node_id, Utc::now())
        .map_err(internal_error)?
        .ok_or_else(|| not_found(format!(
            "SmartSentinel telemetry not found for project '{}' node '{}'",
            project_id, node_id
        )))?;

    Ok(JsonResponse(latest_response(&observation)))
}

pub async fn list_sentinel_telemetry(
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = project_id(&headers)?;
    let observations =
        latest_telemetry_by_project(&project_id, Utc::now()).map_err(internal_error)?;

    Ok(JsonResponse(json!({
        "schema_version": "0.1.0",
        "project_id": project_id,
        "count": observations.len(),
        "history_capacity": DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        "observations": observations,
        "side_effects_performed": false
    })))
}

pub async fn get_sentinel_telemetry_history(
    Path(node_id): Path<String>,
    Query(query): Query<TelemetryHistoryQuery>,
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = project_id(&headers)?;
    let limit = query.limit.unwrap_or(50).min(DEFAULT_TELEMETRY_HISTORY_CAPACITY);
    if limit == 0 {
        return Err(bad_request("telemetry history limit must be greater than zero".to_string()));
    }
    let observations =
        telemetry_history(&project_id, &node_id, limit, Utc::now()).map_err(internal_error)?;

    Ok(JsonResponse(json!({
        "schema_version": "0.1.0",
        "project_id": project_id,
        "node_id": node_id,
        "count": observations.len(),
        "limit": limit,
        "history_capacity": DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        "observations": observations,
        "side_effects_performed": false
    })))
}

pub async fn get_observed_dgraph(
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = project_id(&headers)?;
    let graph = operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found(format!(
            "operational D-Graph not found for project '{}'",
            project_id
        )))?;
    let telemetry = latest_telemetry_by_project(&project_id, Utc::now())
        .map_err(internal_error)?;

    Ok(JsonResponse(build_observed_dgraph(&graph, telemetry)))
}

pub(crate) fn build_observed_dgraph(
    graph: &OperationalDGraph,
    telemetry: Vec<TelemetryObservationView>,
) -> Value {
    let telemetry_by_node = telemetry
        .into_iter()
        .map(|observation| (observation.node_id.clone(), observation))
        .collect::<std::collections::BTreeMap<_, _>>();

    let nodes = graph
        .nodes
        .iter()
        .map(|node| {
            let observed = telemetry_by_node
                .get(&node.node_id)
                .map(|observation| {
                    json!({
                        "status": observation.telemetry_freshness,
                        "telemetry_freshness": observation.telemetry_freshness,
                        "telemetry_age_seconds": observation.telemetry_age_seconds,
                        "last_seen_utc": observation.observed_at_utc,
                        "sequence": observation.sequence,
                        "telemetry": observation.payload
                    })
                })
                .unwrap_or_else(|| {
                    json!({
                        "status": "missing",
                        "telemetry_freshness": "missing",
                        "telemetry_age_seconds": Value::Null,
                        "last_seen_utc": Value::Null,
                        "sequence": Value::Null,
                        "telemetry": Value::Null
                    })
                });
            let failure_state = update_failure_state(
                &graph.project_id,
                &node.node_id,
                observed
                    .get("telemetry_freshness")
                    .and_then(Value::as_str)
                    .unwrap_or("missing"),
                observed.get("sequence").and_then(Value::as_u64),
                observed.get("last_seen_utc").and_then(Value::as_str),
                Utc::now(),
            )
            .map(|state| serde_json::to_value(state).unwrap_or_else(|_| json!({
                "state": "unknown",
                "reason": "failure_state_serialization_error"
            })))
            .unwrap_or_else(|error| json!({
                "state": "unknown",
                "reason": error,
                "consecutive_failures": 0,
                "consecutive_successes": 0,
                "transitions": []
            }));

            json!({
                "node_id": node.node_id,
                "desired_services": node.desired_services,
                "observed": observed,
                "failure_state": failure_state
            })
        })
        .collect::<Vec<_>>();

    let configured_relations = graph
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("relations"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let observed_relations = configured_relations
        .iter()
        .map(|relation| {
            let from = relation.get("from").and_then(Value::as_str).unwrap_or("");
            let to = relation.get("to").and_then(Value::as_str).unwrap_or("");
            let from_fresh = telemetry_by_node
                .get(from)
                .is_some_and(|observation| observation.telemetry_freshness == "fresh");
            let to_fresh = telemetry_by_node
                .get(to)
                .is_some_and(|observation| observation.telemetry_freshness == "fresh");
            let status = if from_fresh && to_fresh {
                "available"
            } else if from_fresh || to_fresh {
                "degraded"
            } else {
                "unavailable"
            };
            json!({
                "from": from,
                "to": to,
                "relation": relation.get("rel").cloned().unwrap_or_else(|| json!("configured")),
                "configured": relation,
                "observed_status": status,
                "evidence": "node_telemetry_freshness"
            })
        })
        .collect::<Vec<_>>();

    let measured_connectivity_edges = telemetry_by_node
        .values()
        .filter_map(|observation| {
            let control_plane = observation
                .payload
                .get("network")
                .and_then(|network| network.get("control_plane"))?;
            Some(json!({
                "from": observation.node_id,
                "to": control_plane.get("target").cloned().unwrap_or(Value::Null),
                "relation": "control_plane_reachable",
                "reachable": control_plane.get("reachable").cloned().unwrap_or(Value::Null),
                "rtt_ms": control_plane.get("rtt_ms").cloned().unwrap_or(Value::Null),
                "telemetry_freshness": observation.telemetry_freshness,
                "observed_at_utc": observation.observed_at_utc
            }))
        })
        .collect::<Vec<_>>();

    let measured_peer_edges = telemetry_by_node
        .values()
        .flat_map(|observation| {
            observation
                .payload
                .get("network")
                .and_then(|network| network.get("peers"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(move |peer| {
                    let peer_node_id = peer.get("node_id").and_then(Value::as_str)?;
                    Some(json!({
                        "from": observation.node_id,
                        "to": peer_node_id,
                        "relation": "peer_connectivity",
                        "reachable": peer.get("reachable").cloned().unwrap_or(Value::Null),
                        "rtt_ms": peer.get("rtt_ms").cloned().unwrap_or(Value::Null),
                        "consecutive_failures": peer.get("consecutive_failures").cloned().unwrap_or(Value::Null),
                        "last_success_at_utc": peer.get("last_success_at_utc").cloned().unwrap_or(Value::Null),
                        "error": peer.get("error").cloned().unwrap_or(Value::Null),
                        "telemetry_freshness": observation.telemetry_freshness,
                        "observed_at_utc": observation.observed_at_utc
                    }))
                })
        })
        .collect::<Vec<_>>();

    let all_nodes_fresh = graph.nodes.iter().all(|node| {
        telemetry_by_node
            .get(&node.node_id)
            .is_some_and(|observation| observation.telemetry_freshness == "fresh")
    });

    json!({
        "schema_version": "0.1.0",
        "graph_kind": "telemetry_aware_observed_dgraph",
        "project_id": graph.project_id,
        "dgraph_id": graph.dgraph_id,
        "revision": graph.revision,
        "generated_at_utc": Utc::now(),
        "consistency_model": "single_operational_dgraph_revision_joined_with_latest_node_telemetry",
        "failure_state_machine": {
            "suspect_after_consecutive_failures": 1,
            "unavailable_after_consecutive_failures": 3,
            "recovering_after_first_fresh_observation": 1,
            "healthy_after_consecutive_successes": 3
        },
        "nodes": nodes,
        "configured_relations": configured_relations,
        "observed_relations": observed_relations,
        "measured_connectivity_edges": measured_connectivity_edges,
        "measured_peer_edges": measured_peer_edges,
        "path_status": if all_nodes_fresh { "all_expected_nodes_fresh" } else { "degraded_or_missing_telemetry" },
        "side_effects_performed": false
    })
}

fn latest_response(observation: &TelemetryObservationView) -> Value {
    json!({
        "schema_version": observation.schema_version,
        "project_id": observation.project_id,
        "node_id": observation.node_id,
        "latest": observation,
        "history_count": telemetry_history_count(
            &observation.project_id,
            &observation.node_id
        ).unwrap_or(0),
        "history_capacity": DEFAULT_TELEMETRY_HISTORY_CAPACITY,
        "side_effects_performed": false
    })
}

fn project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| bad_request("missing or invalid X-Datum-Project header".to_string()))
}

fn bad_request(message: String) -> ApiError {
    (StatusCode::BAD_REQUEST, JsonResponse(json!({"error": message})))
}

fn not_found(message: String) -> ApiError {
    (StatusCode::NOT_FOUND, JsonResponse(json!({"error": message})))
}

fn internal_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observed_graph_marks_missing_nodes_without_fabricating_telemetry() {
        let graph = OperationalDGraph {
            schema_version: "0.1.0".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            revision: 1,
            project_id: "project-a".to_string(),
            nodes: vec![crate::core::operational_dgraph::OperationalNodeDesiredState {
                node_id: "fog-01".to_string(),
                desired_services: Vec::new(),
            }],
            metadata: None,
        };

        let value = build_observed_dgraph(&graph, Vec::new());
        assert_eq!(value["nodes"][0]["observed"]["status"], "missing");
        assert_eq!(value["path_status"], "degraded_or_missing_telemetry");
        assert!(!value["side_effects_performed"].as_bool().unwrap());
    }
}
