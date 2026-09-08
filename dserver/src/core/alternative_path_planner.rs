use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::core::sentinel_telemetry::{
    TelemetryObservationView, DEFAULT_TELEMETRY_HISTORY_CAPACITY,
};

const DEFAULT_MAX_HOPS: usize = 4;
const MAX_MAX_HOPS: usize = 8;

pub fn build_alternative_path_what_if_context(
    observed_dgraph: &Value,
    history_by_node: BTreeMap<String, Vec<TelemetryObservationView>>,
    requested_from: Option<&str>,
    requested_to: Option<&str>,
    requested_max_hops: Option<usize>,
) -> Result<Value, String> {
    let node_ids = observed_node_ids(observed_dgraph)?;
    let from = normalize_optional_node(requested_from)?;
    let to = normalize_optional_node(requested_to)?;

    if let Some(node_id) = from.as_deref() {
        if !node_ids.iter().any(|known| known == node_id) {
            return Err(format!(
                "alternative path source node '{}' is not present in observed D-Graph",
                node_id
            ));
        }
    }
    if let Some(node_id) = to.as_deref() {
        if !node_ids.iter().any(|known| known == node_id) {
            return Err(format!(
                "alternative path target node '{}' is not present in observed D-Graph",
                node_id
            ));
        }
    }

    let max_hops = requested_max_hops
        .unwrap_or(DEFAULT_MAX_HOPS)
        .clamp(1, MAX_MAX_HOPS);

    let mut node_series = Vec::with_capacity(node_ids.len());
    let mut feature_series = Vec::new();
    let mut history_observation_count = 0usize;

    for node_id in &node_ids {
        let observations = history_by_node.get(node_id).cloned().unwrap_or_default();
        history_observation_count += observations.len();
        feature_series.extend(
            observations
                .iter()
                .map(|observation| feature_row(node_id, observation)),
        );
        node_series.push(json!({
            "node_id": node_id,
            "observation_count": observations.len(),
            "observations": observations,
            "side_effects_performed": false
        }));
    }

    let measured_peer_edge_count = observed_dgraph
        .get("measured_peer_edges")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let configured_relation_count = observed_dgraph
        .get("configured_relations")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    Ok(json!({
        "schema_version": "0.1.0",
        "contract_kind": "datum_alternative_path_what_if_context",
        "purpose": "external_ai_historical_projection",
        "project_id": observed_dgraph.get("project_id").cloned().unwrap_or(Value::Null),
        "dgraph_id": observed_dgraph.get("dgraph_id").cloned().unwrap_or(Value::Null),
        "dgraph_revision": observed_dgraph.get("revision").cloned().unwrap_or(Value::Null),
        "observed_path_status": observed_dgraph.get("path_status").cloned().unwrap_or(Value::Null),
        "observed_dgraph": observed_dgraph,
        "requested_scope": {
            "from": from,
            "to": to,
            "max_hops": max_hops
        },
        "history": {
            "capacity": DEFAULT_TELEMETRY_HISTORY_CAPACITY,
            "node_series": node_series,
            "node_series_count": node_ids.len(),
            "observation_count": history_observation_count
        },
        "feature_schema": {
            "normalization": "none",
            "source": "raw_typed_smart_sentinel_telemetry",
            "features": [
                "cpu_usage_percent",
                "load_1",
                "memory_available_bytes",
                "process_count",
                "control_plane_rtt_ms",
                "peer_reachability",
                "peer_rtt_ms",
                "peer_consecutive_failures"
            ]
        },
        "feature_series": feature_series,
        "baseline": {
            "configured_relation_count": configured_relation_count,
            "measured_peer_edge_count": measured_peer_edge_count,
            "source": "current_observed_dgraph"
        },
        "inference": {
            "mode": "external_ai_pending",
            "model_kind": "not_executed",
            "training_performed": false,
            "prediction_performed": false,
            "projection_available": false,
            "required_input": "historical_node_series_and_observed_topology"
        },
        "projections": [],
        "summary": {
            "expected_node_count": node_ids.len(),
            "node_series_count": node_ids.len(),
            "observation_count": history_observation_count,
            "feature_row_count": feature_series.len(),
            "projection_count": 0
        },
        "what_if": {
            "mode": "historical_projection_input",
            "changes_applied": false,
            "execution_allowed": false,
            "side_effects_performed": false
        },
        "side_effects_performed": false
    }))
}

fn observed_node_ids(observed_dgraph: &Value) -> Result<Vec<String>, String> {
    let nodes = observed_dgraph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "observed D-Graph nodes must be an array".to_string())?;

    let mut ids = Vec::with_capacity(nodes.len());
    for node in nodes {
        let node_id = node
            .get("node_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "observed D-Graph node_id must be a non-empty string".to_string())?;
        if ids.iter().any(|known| known == node_id) {
            return Err(format!("duplicate observed D-Graph node '{}'", node_id));
        }
        ids.push(node_id.to_string());
    }
    Ok(ids)
}

fn normalize_optional_node(value: Option<&str>) -> Result<Option<String>, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.chars().count() > 128 {
                Err("alternative path node filter is too long".to_string())
            } else {
                Ok(value.to_string())
            }
        })
        .transpose()
}

fn feature_row(node_id: &str, observation: &TelemetryObservationView) -> Value {
    let payload = &observation.payload;
    let peer_series = payload
        .get("network")
        .and_then(|network| network.get("peers"))
        .and_then(Value::as_array)
        .map(|peers| {
            peers
                .iter()
                .filter_map(|peer| {
                    let peer_node_id = peer.get("node_id").and_then(Value::as_str)?;
                    Some(json!({
                        "peer_node_id": peer_node_id,
                        "reachable": peer.get("reachable").cloned().unwrap_or(Value::Null),
                        "rtt_ms": peer.get("rtt_ms").cloned().unwrap_or(Value::Null),
                        "consecutive_failures": peer.get("consecutive_failures").cloned().unwrap_or(Value::Null)
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "node_id": node_id,
        "sequence": observation.sequence,
        "cycle_id": payload.get("cycle_id").cloned().unwrap_or(Value::Null),
        "observed_at_utc": observation.observed_at_utc,
        "telemetry_freshness": observation.telemetry_freshness,
        "features": {
            "cpu_usage_percent": nested_value(payload, &["host", "cpu", "usage_percent"]),
            "load_1": nested_value(payload, &["host", "cpu", "load_1"]),
            "memory_available_bytes": nested_value(payload, &["host", "memory", "available_bytes"]),
            "process_count": nested_value(payload, &["host", "process_count"]),
            "control_plane_rtt_ms": nested_value(payload, &["network", "control_plane", "rtt_ms"]),
            "peer_reachability": peer_series
                .iter()
                .map(|peer| peer.get("reachable").cloned().unwrap_or(Value::Null))
                .collect::<Vec<_>>(),
            "peer_rtt_ms": peer_series
                .iter()
                .map(|peer| peer.get("rtt_ms").cloned().unwrap_or(Value::Null))
                .collect::<Vec<_>>(),
            "peer_consecutive_failures": peer_series
                .iter()
                .map(|peer| peer.get("consecutive_failures").cloned().unwrap_or(Value::Null))
                .collect::<Vec<_>>()
        },
        "peers": peer_series,
        "side_effects_performed": false
    })
}

fn nested_value(payload: &Value, path: &[&str]) -> Value {
    path.iter()
        .try_fold(payload, |value, key| value.get(*key))
        .cloned()
        .unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn observation(node_id: &str, sequence: u64, cpu: f64) -> TelemetryObservationView {
        TelemetryObservationView {
            schema_version: "0.1.0".to_string(),
            project_id: "monitor-f49c8".to_string(),
            node_id: node_id.to_string(),
            sentinel_instance_id: format!("{node_id}-sentinel"),
            sequence,
            observed_at_utc: format!("2026-09-08T00:00:{sequence:02}Z"),
            received_at_utc: format!("2026-09-08T00:00:{sequence:02}Z"),
            telemetry_age_seconds: 1,
            telemetry_freshness: "fresh".to_string(),
            payload: json!({
                "cycle_id": format!("cycle-{sequence}"),
                "host": {
                    "cpu": {"usage_percent": cpu, "load_1": 0.5},
                    "memory": {"available_bytes": 1024},
                    "process_count": 10
                },
                "network": {
                    "control_plane": {"rtt_ms": 2.0},
                    "peers": [{
                        "node_id": "cloud-01",
                        "reachable": true,
                        "rtt_ms": 3.0,
                        "consecutive_failures": 0
                    }]
                }
            }),
            side_effects_performed: false,
        }
    }

    fn graph() -> Value {
        json!({
            "project_id": "monitor-f49c8",
            "dgraph_id": "dgraph:test",
            "revision": 5,
            "path_status": "all_expected_nodes_fresh",
            "nodes": [
                {"node_id": "fog-01"},
                {"node_id": "cloud-01"}
            ],
            "configured_relations": [{"from": "fog-01", "to": "cloud-01"}],
            "measured_peer_edges": [{"from": "fog-01", "to": "cloud-01"}]
        })
    }

    #[test]
    fn builds_historical_input_without_predicting() {
        let mut history = BTreeMap::new();
        history.insert(
            "fog-01".to_string(),
            vec![observation("fog-01", 1, 12.0), observation("fog-01", 2, 18.0)],
        );

        let context = build_alternative_path_what_if_context(
            &graph(),
            history,
            Some("fog-01"),
            Some("cloud-01"),
            None,
        )
        .unwrap();

        assert_eq!(context["contract_kind"], "datum_alternative_path_what_if_context");
        assert_eq!(context["history"]["observation_count"], 2);
        assert_eq!(context["feature_series"][0]["features"]["cpu_usage_percent"], 12.0);
        assert_eq!(context["inference"]["prediction_performed"], false);
        assert_eq!(context["projections"].as_array().unwrap().len(), 0);
        assert_eq!(context["side_effects_performed"], false);
    }

    #[test]
    fn preserves_empty_series_for_expected_nodes() {
        let context = build_alternative_path_what_if_context(
            &graph(),
            BTreeMap::new(),
            None,
            None,
            Some(99),
        )
        .unwrap();

        assert_eq!(context["history"]["node_series_count"], 2);
        assert_eq!(context["history"]["node_series"][1]["observation_count"], 0);
        assert_eq!(context["requested_scope"]["max_hops"], 8);
        assert_eq!(context["summary"]["projection_count"], 0);
    }

    #[test]
    fn rejects_unknown_scope_node() {
        let error = build_alternative_path_what_if_context(
            &graph(),
            BTreeMap::new(),
            Some("unknown"),
            Some("cloud-01"),
            None,
        )
        .unwrap_err();

        assert!(error.contains("not present in observed D-Graph"));
    }

    #[test]
    fn feature_rows_keep_raw_typed_payload_available() {
        let mut history = BTreeMap::new();
        history.insert(
            "fog-01".to_string(),
            vec![observation("fog-01", 1, 12.0)],
        );
        let context = build_alternative_path_what_if_context(
            &graph(),
            history,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            context["feature_series"][0]["peers"][0]["peer_node_id"],
            "cloud-01"
        );
        assert_eq!(
            context["history"]["node_series"][0]["observations"][0]["payload"]["host"]["cpu"]["load_1"],
            0.5
        );
    }

    #[test]
    fn timestamp_and_sequence_are_kept_for_reproducibility() {
        let mut history = BTreeMap::new();
        history.insert(
            "fog-01".to_string(),
            vec![observation("fog-01", 4, 22.0)],
        );
        let context = build_alternative_path_what_if_context(
            &graph(),
            history,
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(context["feature_series"][0]["sequence"], 4);
        assert_eq!(context["feature_series"][0]["cycle_id"], "cycle-4");
        assert_eq!(
            context["feature_series"][0]["observed_at_utc"],
            "2026-09-08T00:00:04Z"
        );
    }

    #[test]
    fn current_graph_is_preserved_for_external_model() {
        let context = build_alternative_path_what_if_context(
            &graph(),
            BTreeMap::new(),
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(context["observed_dgraph"]["revision"], 5);
        assert_eq!(context["baseline"]["measured_peer_edge_count"], 1);
    }

    #[allow(dead_code)]
    fn _utc_typecheck() {
        let _ = Utc.with_ymd_and_hms(2026, 9, 8, 0, 0, 0).unwrap();
    }
}
