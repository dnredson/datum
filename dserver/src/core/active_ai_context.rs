use std::collections::{BTreeMap, HashSet};

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    model::SnapshotEnvelope,
    operational_analysis_context::{
        OperationalAnalysisContext, COMPARISON_UNKNOWN, TELEMETRY_FRESHNESS_FRESH,
        TELEMETRY_FRESHNESS_MISSING, TELEMETRY_FRESHNESS_STALE,
    },
    operational_dgraph::{validate_operational_dgraph, OperationalDGraph},
};

pub const ACTIVE_AI_CONTEXT_SCHEMA_VERSION: &str = "0.1.0";
pub const ACTIVE_AI_CONTEXT_CONSISTENCY: &str =
    "single_accepted_dgraph_revision_non_atomic_latest_per_node";

/// A transport-neutral, lossless evidence package, not a model-specific feature tensor.
/// The original Firestore documents remain in analysis_context.nodes[].latest_telemetry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveAiContext {
    pub schema_version: String,
    pub acquisition_mode: String,
    pub project_id: String,
    pub generated_at_utc: String,
    pub consistency_model: String,
    pub operational_dgraph: OperationalDGraph,
    pub analysis_context: OperationalAnalysisContext,
    pub node_snapshots: Vec<AiNodeSnapshot>,
    pub evidence_summary: AiEvidenceSummary,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiNodeSnapshot {
    pub node_id: String,
    pub snapshot_status: String,
    pub telemetry_freshness: String,
    pub telemetry_age_seconds: Option<u64>,
    pub cycle_id: Option<String>,
    pub observed_at_utc: Option<String>,
    pub usable_for_inference: bool,
    /// The entire decoded SnapshotEnvelope, including all entities, edges,
    /// events, actions and any additional JSON fields. Never a fabricated snapshot.
    pub snapshot_envelope: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiEvidenceSummary {
    pub expected_node_count: u64,
    pub valid_snapshot_count: u64,
    pub usable_node_count: u64,
    pub fresh_node_count: u64,
    pub stale_node_count: u64,
    pub missing_node_count: u64,
    pub invalid_snapshot_count: u64,
    /// Completeness means valid, fresh evidence for every desired node;
    /// it does not mean all services are converged or healthy.
    pub complete: bool,
}

pub fn build_active_ai_context(
    graph: OperationalDGraph,
    mut analysis: OperationalAnalysisContext,
) -> Result<ActiveAiContext, String> {
    validate_operational_dgraph(&graph)?;
    if analysis.acquisition_mode != "active"
        || analysis.project_id != graph.project_id
        || analysis.dgraph_id != graph.dgraph_id
        || analysis.dgraph_revision != graph.revision
        || analysis.node_count != graph.nodes.len() as u64
        || analysis.nodes.len() != graph.nodes.len()
        || analysis.side_effects_performed
    {
        return Err("active AI context graph/assessment identity or cardinality mismatch".into());
    }

    let expected: HashSet<&str> = graph.nodes.iter().map(|n| n.node_id.as_str()).collect();
    let mut seen = HashSet::new();
    for node in &analysis.nodes {
        if !expected.contains(node.node_id.as_str()) || !seen.insert(node.node_id.as_str()) {
            return Err(format!("unexpected or duplicate assessment node '{}'", node.node_id));
        }
    }

    // Preserve the accepted graph's order, not the order in which network reads finish.
    let mut by_node = BTreeMap::new();
    for node in &analysis.nodes {
        by_node.insert(node.node_id.clone(), node.clone());
    }
    let mut snapshots = Vec::with_capacity(graph.nodes.len());
    let mut summary = AiEvidenceSummary {
        expected_node_count: graph.nodes.len() as u64,
        valid_snapshot_count: 0,
        usable_node_count: 0,
        fresh_node_count: 0,
        stale_node_count: 0,
        missing_node_count: 0,
        invalid_snapshot_count: 0,
        complete: false,
    };

    for desired_node in &graph.nodes {
        let node = by_node.get(&desired_node.node_id).expect("validated node set");
        let freshness = node.telemetry_freshness.as_str();
        match freshness {
            TELEMETRY_FRESHNESS_FRESH => summary.fresh_node_count += 1,
            TELEMETRY_FRESHNESS_STALE => summary.stale_node_count += 1,
            TELEMETRY_FRESHNESS_MISSING => summary.missing_node_count += 1,
            _ => return Err(format!("unsupported telemetry freshness for '{}'", node.node_id)),
        }

        let mut entry = AiNodeSnapshot {
            node_id: node.node_id.clone(),
            snapshot_status: "missing".into(),
            telemetry_freshness: node.telemetry_freshness.clone(),
            telemetry_age_seconds: node.telemetry_age_seconds,
            cycle_id: None,
            observed_at_utc: node.telemetry_observed_at_utc.clone(),
            usable_for_inference: false,
            snapshot_envelope: None,
            error: node.telemetry_error.clone(),
        };

        if let Some(document) = &node.latest_telemetry {
            match decode_snapshot_document(document, &graph.project_id, &node.node_id) {
                Ok((envelope, cycle_id, observed_at)) => {
                    entry.snapshot_status = "available".into();
                    entry.cycle_id = Some(cycle_id);
                    entry.observed_at_utc = Some(observed_at.clone());
                    entry.snapshot_envelope = Some(envelope);
                    // The assessment and exported payload must refer to the same instant.
                    if !node
                        .telemetry_observed_at_utc
                        .as_deref()
                        .is_some_and(|assessment_timestamp| {
                            same_instant(assessment_timestamp, &observed_at)
                        })
                    {
                        entry.snapshot_status = "invalid".into();
                        entry.error = Some("assessment/payload observation timestamp mismatch".into());
                        entry.snapshot_envelope = None;
                    } else {
                        summary.valid_snapshot_count += 1;
                        entry.usable_for_inference = freshness == TELEMETRY_FRESHNESS_FRESH;
                        if entry.usable_for_inference {
                            summary.usable_node_count += 1;
                        }
                    }
                }
                Err(error) => {
                    entry.snapshot_status = "invalid".into();
                    entry.error = Some(error);
                }
            }
        } else if node.telemetry_available {
            entry.snapshot_status = "invalid".into();
            entry.error = Some("telemetry marked available without a Firestore document".into());
        }

        if entry.snapshot_status == "invalid" {
            summary.invalid_snapshot_count += 1;
            // Do not expose a convergence claim based on malformed or mismatched
            // evidence. Preserve observed attributes and the original raw document.
            if let Some(assessment_node) = analysis.nodes.iter_mut().find(|n| n.node_id == entry.node_id) {
                for comparison in &mut assessment_node.service_comparisons {
                    comparison.comparison_status = COMPARISON_UNKNOWN.into();
                    comparison.reason = "invalid_telemetry".into();
                }
            }
        }
        snapshots.push(entry);
    }

    summary.complete = summary.usable_node_count == summary.expected_node_count;
    Ok(ActiveAiContext {
        schema_version: ACTIVE_AI_CONTEXT_SCHEMA_VERSION.into(),
        acquisition_mode: "active".into(),
        project_id: graph.project_id.clone(),
        generated_at_utc: analysis.generated_at_utc.clone(),
        consistency_model: ACTIVE_AI_CONTEXT_CONSISTENCY.into(),
        operational_dgraph: graph,
        analysis_context: analysis,
        node_snapshots: snapshots,
        evidence_summary: summary,
        side_effects_performed: false,
    })
}

fn string_field<'a>(fields: &'a Value, name: &str) -> Option<&'a str> {
    fields.get(name)?.get("stringValue")?.as_str()
}

fn same_instant(left: &str, right: &str) -> bool {
    match (DateTime::parse_from_rfc3339(left), DateTime::parse_from_rfc3339(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn decode_snapshot_document(
    document: &Value,
    project_id: &str,
    node_id: &str,
) -> Result<(Value, String, String), String> {
    let fields = document.get("fields").ok_or("Firestore document missing fields")?;
    let payload = string_field(fields, "payload").ok_or("Firestore document missing payload")?;
    let value: Value = serde_json::from_str(payload).map_err(|e| format!("invalid snapshot JSON: {e}"))?;
    let envelope: SnapshotEnvelope = serde_json::from_value(value.clone())
        .map_err(|e| format!("invalid SnapshotEnvelope: {e}"))?;
    let meta = &envelope.snapshot.meta;
    if envelope.protocol_version.trim().is_empty()
        || meta.schema_version.trim().is_empty()
        || meta.cycle_id.trim().is_empty()
        || meta.node_id != node_id
        || DateTime::parse_from_rfc3339(&meta.observed_at_utc).is_err()
    {
        return Err("snapshot has invalid protocol, schema, identity or timestamp".into());
    }
    for (name, field) in [
        ("entities", &envelope.snapshot.entities),
        ("edges", &envelope.snapshot.edges),
        ("events", &envelope.snapshot.events),
        ("actions", &envelope.snapshot.actions),
    ] {
        if !field.is_array() {
            return Err(format!("snapshot.{name} must be an array"));
        }
    }
    for (field, expected) in [
        ("project_id", project_id),
        ("node_id", node_id),
        ("cycle_id", meta.cycle_id.as_str()),
    ] {
        if let Some(actual) = string_field(fields, field) {
            if actual != expected {
                return Err(format!("Firestore {field} does not match snapshot identity"));
            }
        }
    }
    if let Some(observed_at) = string_field(fields, "observed_at_utc") {
        if !same_instant(observed_at, &meta.observed_at_utc) {
            return Err("Firestore timestamp does not match snapshot timestamp".into());
        }
    }
    Ok((value, meta.cycle_id.clone(), meta.observed_at_utc.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use crate::core::operational_analysis_context::build_active_analysis_context;
    use crate::core::operational_dgraph::{OperationalDesiredService, OperationalNodeDesiredState};

    fn graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: "0.1.0".into(), dgraph_id: "dgraph:ai-test".into(),
            revision: 4, project_id: "project-a".into(),
            nodes: vec![
                OperationalNodeDesiredState { node_id: "fog-01".into(), desired_services: vec![
                    OperationalDesiredService { service_id: "mqtt".into(), runtime: "container".into(), desired_state: "running".into() },
                ] },
                OperationalNodeDesiredState { node_id: "cloud-01".into(), desired_services: vec![] },
            ],
            metadata: Some(json!({"relations": [{"from":"fog-01","to":"cloud-01","rel":"connected_to","latency_ms":12.5}]})),
        }
    }

    fn document(node: &str, observed_at: &str, running: bool) -> Value {
        let state = if running { "running" } else { "exited" };
        let payload = json!({
            "protocol_version": "s0.v1", "custom_extension": {"preserve": true},
            "snapshot": {
                "meta": {"node_id":node,"cycle_id":format!("cycle-{node}"),"observed_at_utc":observed_at,"schema_version":"s0.v1"},
                "entities": [
                    {"id":"service:mqtt","attrs":{"status":"unknown","binding":{"bound":true},"bindings":[{"bound":true,"target_id":"container:docker:mqtt"}]}},
                    {"id":"container:docker:mqtt","attrs":{"state":state,"cpu":3.5}}
                ],
                "edges": [{"from":"service:mqtt","to":"container:docker:mqtt","rel":"implemented_by","weight":0.7}],
                "events": [{"kind":"sample"}], "actions": []
            }
        });
        json!({"fields":{
            "project_id":{"stringValue":"project-a"},"node_id":{"stringValue":node},
            "cycle_id":{"stringValue":format!("cycle-{node}")},
            "observed_at_utc":{"stringValue":observed_at},
            "payload":{"stringValue":serde_json::to_string(&payload).unwrap()}
        }})
    }

    fn build(docs: BTreeMap<String, Result<Value, String>>) -> ActiveAiContext {
        let graph = graph();
        let analysis = build_active_analysis_context(&graph, docs,
            Utc.with_ymd_and_hms(2026,9,7,12,0,0).unwrap(),120).unwrap();
        build_active_ai_context(graph, analysis).unwrap()
    }

    #[test]
    fn preserves_complete_graph_all_snapshots_and_unknown_json() {
        let mut docs = BTreeMap::new();
        docs.insert("fog-01".into(), Ok(document("fog-01","2026-09-07T11:59:30Z",true)));
        docs.insert("cloud-01".into(), Ok(document("cloud-01","2026-09-07T11:59:30Z",false)));
        let result = build(docs);
        assert_eq!(result.operational_dgraph.metadata, graph().metadata);
        assert_eq!(result.node_snapshots.len(),2);
        assert_eq!(result.evidence_summary.usable_node_count,2);
        assert!(result.evidence_summary.complete);
        assert_eq!(result.node_snapshots[0].snapshot_envelope.as_ref().unwrap()["custom_extension"]["preserve"],true);
        assert_eq!(result.node_snapshots[0].snapshot_envelope.as_ref().unwrap()["snapshot"]["edges"][0]["weight"],0.7);
        assert_eq!(result.node_snapshots[0].snapshot_envelope.as_ref().unwrap()["snapshot"]["events"][0]["kind"],"sample");
        assert_eq!(result.analysis_context.nodes[0].service_comparisons[0].comparison_status,"converged");
        assert!(!result.side_effects_performed);
        let roundtrip: ActiveAiContext = serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
        assert_eq!(roundtrip,result);
    }

    #[test]
    fn stale_and_missing_nodes_remain_present_and_unusable() {
        let mut docs = BTreeMap::new();
        docs.insert("fog-01".into(), Ok(document("fog-01","2026-09-07T11:00:00Z",true)));
        docs.insert("cloud-01".into(), Err("firebase unavailable".into()));
        let result = build(docs);
        assert_eq!(result.evidence_summary.stale_node_count,1);
        assert_eq!(result.evidence_summary.missing_node_count,1);
        assert_eq!(result.evidence_summary.usable_node_count,0);
        assert!(!result.evidence_summary.complete);
        assert_eq!(result.node_snapshots[0].snapshot_status,"available");
        assert!(!result.node_snapshots[0].usable_for_inference);
        assert!(result.node_snapshots[1].snapshot_envelope.is_none());
        assert!(result.node_snapshots[1].error.as_ref().unwrap().contains("firebase unavailable"));
        assert_eq!(result.analysis_context.nodes[0].service_comparisons[0].comparison_status,"unknown");
    }

    #[test]
    fn invalid_payload_cannot_claim_convergence() {
        let mut docs = BTreeMap::new();
        let mut invalid = document("fog-01","2026-09-07T11:59:30Z",true);
        invalid["fields"]["cycle_id"]["stringValue"] = json!("different-cycle");
        docs.insert("fog-01".into(),Ok(invalid.clone()));
        docs.insert("cloud-01".into(),Err("missing".into()));
        let result = build(docs);
        assert_eq!(result.evidence_summary.invalid_snapshot_count,1);
        assert_eq!(result.node_snapshots[0].snapshot_status,"invalid");
        assert!(result.node_snapshots[0].snapshot_envelope.is_none());
        assert!(!result.node_snapshots[0].usable_for_inference);
        assert_eq!(result.analysis_context.nodes[0].service_comparisons[0].comparison_status,"unknown");
        assert_eq!(result.analysis_context.nodes[0].service_comparisons[0].reason,"invalid_telemetry");
        assert_eq!(result.analysis_context.nodes[0].latest_telemetry.as_ref(),Some(&invalid));
    }

    #[test]
    fn equivalent_rfc3339_timestamps_remain_valid() {
        let mut docs = BTreeMap::new();
        let mut equivalent = document("fog-01", "2026-09-07T11:59:30Z", true);
        equivalent["fields"]["observed_at_utc"]["stringValue"] =
            json!("2026-09-07T11:59:30+00:00");
        docs.insert("fog-01".into(), Ok(equivalent));
        docs.insert("cloud-01".into(), Err("missing".into()));

        let result = build(docs);

        assert_eq!(result.evidence_summary.invalid_snapshot_count, 0);
        assert_eq!(result.node_snapshots[0].snapshot_status, "available");
        assert!(result.node_snapshots[0].usable_for_inference);
    }

    #[test]
    fn rejects_mixed_revisions_and_incomplete_node_sets() {
        let graph = graph();
        let mut analysis = build_active_analysis_context(&graph,BTreeMap::new(),
            Utc.with_ymd_and_hms(2026,9,7,12,0,0).unwrap(),120).unwrap();
        analysis.dgraph_revision = 3;
        assert!(build_active_ai_context(graph.clone(),analysis.clone()).is_err());
        analysis.dgraph_revision = 4;
        analysis.nodes.pop();
        assert!(build_active_ai_context(graph,analysis).is_err());
    }

    #[test]
    fn rejects_snapshot_belonging_to_another_node() {
        let mut docs = BTreeMap::new();
        docs.insert("fog-01".into(),Ok(document("cloud-01","2026-09-07T11:59:30Z",true)));
        let result = build(docs);
        assert_eq!(result.node_snapshots[0].snapshot_status,"invalid");
        assert!(!result.node_snapshots[0].usable_for_inference);
    }
}
