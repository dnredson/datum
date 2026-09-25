use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::operational_dgraph::{
    validate_operational_dgraph, OperationalDGraph, OperationalDesiredService,
    OperationalNodeDesiredState,
};

pub const OPERATIONAL_ANALYSIS_CONTEXT_SCHEMA_VERSION: &str = "0.2.0";
pub const ANALYSIS_MODE_PASSIVE: &str = "passive";
pub const ANALYSIS_MODE_ACTIVE: &str = "active";
pub const DEFAULT_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS: u64 = 120;

pub const TELEMETRY_FRESHNESS_FRESH: &str = "fresh";
pub const TELEMETRY_FRESHNESS_STALE: &str = "stale";
pub const TELEMETRY_FRESHNESS_MISSING: &str = "missing";

pub const COMPARISON_CONVERGED: &str = "converged";
pub const COMPARISON_DRIFT: &str = "drift";
pub const COMPARISON_UNKNOWN: &str = "unknown";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalAnalysisContext {
    pub schema_version: String,
    pub acquisition_mode: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub generated_at_utc: String,
    pub freshness_max_age_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_node_id: Option<String>,
    pub node_count: u64,
    pub side_effects_performed: bool,
    pub nodes: Vec<OperationalAnalysisNodeContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalAnalysisNodeContext {
    pub node_id: String,
    pub desired_services: Vec<OperationalDesiredService>,
    pub telemetry_source: String,
    pub telemetry_available: bool,
    pub telemetry_freshness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_age_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_observed_at_utc: Option<String>,
    pub service_comparisons: Vec<OperationalServiceComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_telemetry: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalServiceComparison {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
    pub comparison_status: String,
    pub reason: String,
    pub service_entity_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_service_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_runtime_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_target_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_binding_bound: Option<bool>,
}

pub fn build_passive_analysis_context(
    graph: &OperationalDGraph,
    node_id: &str,
    telemetry: Result<Value, String>,
    now: DateTime<Utc>,
    freshness_max_age_seconds: u64,
) -> Result<OperationalAnalysisContext, String> {
    validate_operational_dgraph(graph)?;
    let node_id = node_id.trim();
    let node = graph
        .nodes
        .iter()
        .find(|node| node.node_id.trim() == node_id)
        .ok_or_else(|| {
            format!(
                "passive analysis trigger node '{}' is not present in active operational D-Graph revision {}",
                node_id, graph.revision
            )
        })?;

    Ok(OperationalAnalysisContext {
        schema_version: OPERATIONAL_ANALYSIS_CONTEXT_SCHEMA_VERSION.to_string(),
        acquisition_mode: ANALYSIS_MODE_PASSIVE.to_string(),
        project_id: graph.project_id.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        generated_at_utc: now.to_rfc3339(),
        freshness_max_age_seconds,
        trigger_node_id: Some(node.node_id.clone()),
        node_count: 1,
        side_effects_performed: false,
        nodes: vec![build_node_context(
            node,
            telemetry,
            now,
            freshness_max_age_seconds,
        )],
    })
}

pub fn build_active_analysis_context(
    graph: &OperationalDGraph,
    mut telemetry_by_node: BTreeMap<String, Result<Value, String>>,
    now: DateTime<Utc>,
    freshness_max_age_seconds: u64,
) -> Result<OperationalAnalysisContext, String> {
    validate_operational_dgraph(graph)?;

    let nodes = graph
        .nodes
        .iter()
        .map(|node| {
            let telemetry = telemetry_by_node.remove(node.node_id.trim()).unwrap_or_else(|| {
                Err(format!(
                    "active analysis telemetry acquisition result missing for node '{}'",
                    node.node_id
                ))
            });
            build_node_context(node, telemetry, now, freshness_max_age_seconds)
        })
        .collect::<Vec<_>>();

    Ok(OperationalAnalysisContext {
        schema_version: OPERATIONAL_ANALYSIS_CONTEXT_SCHEMA_VERSION.to_string(),
        acquisition_mode: ANALYSIS_MODE_ACTIVE.to_string(),
        project_id: graph.project_id.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        generated_at_utc: now.to_rfc3339(),
        freshness_max_age_seconds,
        trigger_node_id: None,
        node_count: nodes.len() as u64,
        side_effects_performed: false,
        nodes,
    })
}

fn build_node_context(
    node: &OperationalNodeDesiredState,
    telemetry: Result<Value, String>,
    now: DateTime<Utc>,
    freshness_max_age_seconds: u64,
) -> OperationalAnalysisNodeContext {
    match telemetry {
        Ok(document) => {
            let observed_at = firestore_observed_at(&document);
            let (telemetry_freshness, telemetry_age_seconds) =
                classify_freshness(observed_at.as_deref(), now, freshness_max_age_seconds);
            let snapshot = firestore_snapshot_payload(&document);
            let service_comparisons = node
                .desired_services
                .iter()
                .map(|desired| {
                    compare_service(
                        desired,
                        snapshot.as_ref(),
                        telemetry_freshness.as_str(),
                    )
                })
                .collect();

            OperationalAnalysisNodeContext {
                node_id: node.node_id.clone(),
                desired_services: node.desired_services.clone(),
                telemetry_source: "firebase".to_string(),
                telemetry_available: true,
                telemetry_freshness,
                telemetry_age_seconds,
                telemetry_observed_at_utc: observed_at,
                service_comparisons,
                latest_telemetry: Some(document),
                telemetry_error: None,
            }
        }
        Err(error) => OperationalAnalysisNodeContext {
            node_id: node.node_id.clone(),
            desired_services: node.desired_services.clone(),
            telemetry_source: "firebase".to_string(),
            telemetry_available: false,
            telemetry_freshness: TELEMETRY_FRESHNESS_MISSING.to_string(),
            telemetry_age_seconds: None,
            telemetry_observed_at_utc: None,
            service_comparisons: node
                .desired_services
                .iter()
                .map(|desired| OperationalServiceComparison {
                    service_id: desired.service_id.clone(),
                    runtime: desired.runtime.clone(),
                    desired_state: desired.desired_state.clone(),
                    comparison_status: COMPARISON_UNKNOWN.to_string(),
                    reason: "missing_telemetry".to_string(),
                    service_entity_present: false,
                    observed_service_status: None,
                    observed_runtime_state: None,
                    observed_target_id: None,
                    observed_binding_bound: None,
                })
                .collect(),
            latest_telemetry: None,
            telemetry_error: Some(error),
        },
    }
}

fn classify_freshness(
    observed_at_utc: Option<&str>,
    now: DateTime<Utc>,
    freshness_max_age_seconds: u64,
) -> (String, Option<u64>) {
    let Some(observed_at_utc) = observed_at_utc else {
        return (TELEMETRY_FRESHNESS_MISSING.to_string(), None);
    };
    let Ok(observed_at) = DateTime::parse_from_rfc3339(observed_at_utc) else {
        return (TELEMETRY_FRESHNESS_MISSING.to_string(), None);
    };

    let age_seconds = now
        .signed_duration_since(observed_at.with_timezone(&Utc))
        .num_seconds()
        .max(0) as u64;
    let freshness = if age_seconds <= freshness_max_age_seconds {
        TELEMETRY_FRESHNESS_FRESH
    } else {
        TELEMETRY_FRESHNESS_STALE
    };
    (freshness.to_string(), Some(age_seconds))
}

fn compare_service(
    desired: &OperationalDesiredService,
    snapshot: Option<&Value>,
    telemetry_freshness: &str,
) -> OperationalServiceComparison {
    let observation = snapshot.and_then(|snapshot| service_observation(snapshot, &desired.service_id));
    let service_entity_present = observation.is_some();
    let observed_service_status = observation
        .as_ref()
        .and_then(|observation| observation.service_status.clone());
    let observed_runtime_state = observation
        .as_ref()
        .and_then(|observation| observation.runtime_state.clone());
    let observed_target_id = observation
        .as_ref()
        .and_then(|observation| observation.target_id.clone());
    let observed_binding_bound = observation
        .as_ref()
        .and_then(|observation| observation.binding_bound);

    let (comparison_status, reason) = if telemetry_freshness == TELEMETRY_FRESHNESS_MISSING {
        (COMPARISON_UNKNOWN, "missing_telemetry")
    } else if telemetry_freshness == TELEMETRY_FRESHNESS_STALE {
        (COMPARISON_UNKNOWN, "stale_telemetry")
    } else if observation.is_none() {
        if desired.desired_state == "stopped" {
            (COMPARISON_CONVERGED, "service_not_observed_and_desired_stopped")
        } else {
            (COMPARISON_DRIFT, "desired_service_not_observed")
        }
    } else {
        classify_desired_observed(
            &desired.desired_state,
            observed_runtime_state.as_deref(),
            observed_service_status.as_deref(),
            observed_binding_bound,
        )
    };

    OperationalServiceComparison {
        service_id: desired.service_id.clone(),
        runtime: desired.runtime.clone(),
        desired_state: desired.desired_state.clone(),
        comparison_status: comparison_status.to_string(),
        reason: reason.to_string(),
        service_entity_present,
        observed_service_status,
        observed_runtime_state,
        observed_target_id,
        observed_binding_bound,
    }
}

fn classify_desired_observed(
    desired_state: &str,
    runtime_state: Option<&str>,
    service_status: Option<&str>,
    binding_bound: Option<bool>,
) -> (&'static str, &'static str) {
    let runtime_state = runtime_state.unwrap_or("").to_ascii_lowercase();
    let service_status = service_status.unwrap_or("").to_ascii_lowercase();

    match desired_state.to_ascii_lowercase().as_str() {
        "running" => {
            if runtime_state == "running"
                || matches!(service_status.as_str(), "running" | "available" | "healthy")
            {
                (COMPARISON_CONVERGED, "observed_runtime_running")
            } else if matches!(
                runtime_state.as_str(),
                "stopped" | "exited" | "dead" | "failed"
            ) || matches!(service_status.as_str(), "unavailable" | "failed")
                || binding_bound == Some(false)
            {
                (COMPARISON_DRIFT, "desired_running_but_observed_unavailable")
            } else {
                (COMPARISON_UNKNOWN, "observed_state_not_decisive")
            }
        }
        "stopped" => {
            if runtime_state == "running"
                || matches!(service_status.as_str(), "running" | "available" | "healthy")
            {
                (COMPARISON_DRIFT, "desired_stopped_but_observed_running")
            } else if matches!(
                runtime_state.as_str(),
                "stopped" | "exited" | "dead" | "failed"
            ) || matches!(service_status.as_str(), "unavailable" | "stopped")
            {
                (COMPARISON_CONVERGED, "observed_runtime_not_running")
            } else {
                (COMPARISON_UNKNOWN, "observed_state_not_decisive")
            }
        }
        _ => (COMPARISON_UNKNOWN, "unsupported_desired_state"),
    }
}

#[derive(Debug)]
struct ServiceObservation {
    service_status: Option<String>,
    runtime_state: Option<String>,
    target_id: Option<String>,
    binding_bound: Option<bool>,
}

fn service_observation(snapshot: &Value, service_id: &str) -> Option<ServiceObservation> {
    let entities = snapshot.get("snapshot")?.get("entities")?.as_array()?;
    let service_entity_id = format!("service:{service_id}");
    let service_entity = entities
        .iter()
        .find(|entity| entity.get("id").and_then(Value::as_str) == Some(service_entity_id.as_str()))?;
    let attrs = service_entity.get("attrs")?;
    let service_status = attrs
        .get("status")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let binding_bound = attrs
        .get("binding")
        .and_then(|binding| binding.get("bound"))
        .and_then(Value::as_bool);
    let target_id = attrs
        .get("bindings")
        .and_then(Value::as_array)
        .and_then(|bindings| {
            bindings.iter().find_map(|binding| {
                if binding.get("bound").and_then(Value::as_bool) == Some(true) {
                    binding
                        .get("target_id")
                        .and_then(Value::as_str)
                        .map(ToString::to_string)
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            snapshot
                .get("snapshot")
                .and_then(|snapshot| snapshot.get("edges"))
                .and_then(Value::as_array)
                .and_then(|edges| {
                    edges.iter().find_map(|edge| {
                        if edge.get("from").and_then(Value::as_str)
                            == Some(service_entity_id.as_str())
                            && edge.get("rel").and_then(Value::as_str) == Some("implemented_by")
                        {
                            edge.get("to")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        } else {
                            None
                        }
                    })
                })
        });
    let runtime_state = target_id.as_deref().and_then(|target_id| {
        entities
            .iter()
            .find(|entity| entity.get("id").and_then(Value::as_str) == Some(target_id))
            .and_then(|entity| entity.get("attrs"))
            .and_then(|attrs| {
                attrs
                    .get("state")
                    .or_else(|| attrs.get("status"))
                    .and_then(Value::as_str)
            })
            .map(ToString::to_string)
    });

    Some(ServiceObservation {
        service_status,
        runtime_state,
        target_id,
        binding_bound,
    })
}

fn firestore_observed_at(document: &Value) -> Option<String> {
    document
        .get("fields")
        .and_then(|fields| fields.get("observed_at_utc"))
        .and_then(|value| value.get("stringValue"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn firestore_snapshot_payload(document: &Value) -> Option<Value> {
    let payload = document
        .get("fields")?
        .get("payload")?
        .get("stringValue")?
        .as_str()?;
    serde_json::from_str(payload).ok()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::core::operational_dgraph::{OperationalDesiredService, OperationalNodeDesiredState};

    fn graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: "0.1.0".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            revision: 4,
            project_id: "project-a".to_string(),
            nodes: vec![
                OperationalNodeDesiredState {
                    node_id: "fog-01".to_string(),
                    desired_services: vec![OperationalDesiredService {
                        service_id: "mqtt".to_string(),
                        runtime: "container".to_string(),
                        desired_state: "running".to_string(),
                        application_id: String::new(),
                        dmap_ref: None,
                    }],
                },
                OperationalNodeDesiredState {
                    node_id: "cloud-01".to_string(),
                    desired_services: Vec::new(),
                },
            ],
            metadata: None,
        }
    }

    fn telemetry(observed_at: &str, runtime_state: &str) -> Value {
        let payload = json!({
            "snapshot": {
                "entities": [
                    {
                        "id": "service:mqtt",
                        "attrs": {
                            "status": "unknown",
                            "binding": {"bound": true},
                            "bindings": [
                                {"bound": true, "target_id": "container:docker:mqtt"}
                            ]
                        }
                    },
                    {
                        "id": "container:docker:mqtt",
                        "attrs": {"state": runtime_state}
                    }
                ],
                "edges": []
            }
        });
        json!({
            "fields": {
                "observed_at_utc": {"stringValue": observed_at},
                "payload": {"stringValue": serde_json::to_string(&payload).unwrap()}
            }
        })
    }

    #[test]
    fn fresh_running_runtime_converges_with_desired_running() {
        let context = build_passive_analysis_context(
            &graph(),
            "fog-01",
            Ok(telemetry("2026-09-06T12:00:00Z", "running")),
            Utc.with_ymd_and_hms(2026, 9, 6, 12, 1, 0).unwrap(),
            120,
        )
        .unwrap();

        assert_eq!(context.nodes[0].telemetry_freshness, "fresh");
        assert_eq!(context.nodes[0].telemetry_age_seconds, Some(60));
        assert_eq!(
            context.nodes[0].service_comparisons[0].comparison_status,
            "converged"
        );
        assert_eq!(
            context.nodes[0].service_comparisons[0]
                .observed_runtime_state
                .as_deref(),
            Some("running")
        );
        assert!(!context.side_effects_performed);
    }

    #[test]
    fn stale_running_runtime_is_not_claimed_as_current_convergence() {
        let context = build_passive_analysis_context(
            &graph(),
            "fog-01",
            Ok(telemetry("2026-09-06T11:00:00Z", "running")),
            Utc.with_ymd_and_hms(2026, 9, 6, 12, 1, 0).unwrap(),
            120,
        )
        .unwrap();

        assert_eq!(context.nodes[0].telemetry_freshness, "stale");
        assert_eq!(
            context.nodes[0].service_comparisons[0].comparison_status,
            "unknown"
        );
        assert_eq!(
            context.nodes[0].service_comparisons[0].reason,
            "stale_telemetry"
        );
        assert_eq!(
            context.nodes[0].service_comparisons[0]
                .observed_runtime_state
                .as_deref(),
            Some("running")
        );
    }

    #[test]
    fn missing_telemetry_keeps_desired_state_but_comparison_unknown() {
        let mut telemetry_by_node = BTreeMap::new();
        telemetry_by_node.insert("fog-01".to_string(), Err("firebase unavailable".to_string()));
        telemetry_by_node.insert("cloud-01".to_string(), Err("firebase unavailable".to_string()));

        let context = build_active_analysis_context(
            &graph(),
            telemetry_by_node,
            Utc.with_ymd_and_hms(2026, 9, 6, 12, 1, 0).unwrap(),
            120,
        )
        .unwrap();

        assert_eq!(context.nodes[0].telemetry_freshness, "missing");
        assert_eq!(
            context.nodes[0].service_comparisons[0].comparison_status,
            "unknown"
        );
        assert_eq!(
            context.nodes[0].service_comparisons[0].reason,
            "missing_telemetry"
        );
        assert!(!context.side_effects_performed);
    }
}
