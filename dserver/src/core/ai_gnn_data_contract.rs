use std::collections::{BTreeMap, HashSet};

use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    active_ai_context::{
        ActiveAiContext, AiNodeSnapshot, ACTIVE_AI_CONTEXT_SCHEMA_VERSION,
        ACTIVE_AI_CONTEXT_CONSISTENCY,
    },
    operational_dgraph::{validate_operational_dgraph, OperationalDesiredService},
    operational_dgraph_acquisition::build_explicit_topology,
};

pub const AI_GNN_DATA_CONTRACT_SCHEMA_VERSION: &str = "0.1.0";
pub const AI_GNN_DATA_CONTRACT_KIND: &str = "datum_ai_gnn_input";
pub const AI_GNN_FEATURE_SCHEMA_VERSION: &str = "0.1.0";
pub const AI_GNN_NORMALIZATION_METHOD: &str = "none";
pub const AI_GNN_EXECUTION_SEMANTICS: &str = "proposal_only_no_direct_execution";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnDataContract {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub generated_at_utc: String,
    pub feature_schema: AiGnnFeatureSchema,
    pub topology: AiGnnTopology,
    pub node_features: Vec<AiGnnNodeFeatures>,
    pub edge_features: Vec<AiGnnEdgeFeatures>,
    pub evidence: Vec<AiGnnNodeEvidence>,
    pub inference_eligibility: AiGnnInferenceEligibility,
    pub provenance: AiGnnProvenance,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnFeatureSchema {
    pub schema_version: String,
    pub node_numeric_features: Vec<String>,
    pub node_categorical_features: Vec<String>,
    pub edge_numeric_features: Vec<String>,
    pub edge_categorical_features: Vec<String>,
    pub labels: Vec<String>,
    pub normalization: AiGnnNormalization,
    pub missing_value_policy: String,
    pub label_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiGnnNormalization {
    pub method: String,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnTopology {
    pub nodes: Vec<AiGnnTopologyNode>,
    pub edges: Vec<AiGnnTopologyEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnTopologyNode {
    pub node_id: String,
    pub node_type: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnTopologyEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub edge_type: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnNodeFeatures {
    pub node_id: String,
    pub numeric: BTreeMap<String, Option<f64>>,
    pub numeric_mask: BTreeMap<String, bool>,
    pub categorical: BTreeMap<String, Option<String>>,
    pub categorical_mask: BTreeMap<String, bool>,
    pub labels: Vec<String>,
    pub label_mask: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnEdgeFeatures {
    pub from: String,
    pub to: String,
    pub relation: String,
    pub numeric: BTreeMap<String, Option<f64>>,
    pub numeric_mask: BTreeMap<String, bool>,
    pub categorical: BTreeMap<String, Option<String>>,
    pub categorical_mask: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnNodeEvidence {
    pub node_id: String,
    pub telemetry_freshness: String,
    pub telemetry_age_seconds: Option<u64>,
    pub usable_for_inference: bool,
    pub cycle_id: Option<String>,
    pub observed_at_utc: Option<String>,
    pub snapshot_status: String,
    pub snapshot_envelope: Option<Value>,
    pub desired_services: Vec<AiGnnDesiredService>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiGnnDesiredService {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiGnnInferenceEligibility {
    pub eligible: bool,
    pub reason: String,
    pub expected_node_count: u64,
    pub usable_node_count: u64,
    pub complete_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiGnnProvenance {
    pub source_ai_context_schema_version: String,
    pub source_dgraph_acquisition_schema_version: String,
    pub consistency_model: String,
    pub complete_evidence_required_for_inference: bool,
    pub execution_semantics: String,
}

pub fn build_ai_gnn_data_contract(
    context: ActiveAiContext,
) -> Result<AiGnnDataContract, String> {
    validate_source_context(&context)?;

    let topology = build_explicit_topology(&context.operational_dgraph)?;
    let topology = AiGnnTopology {
        nodes: topology
            .nodes
            .into_iter()
            .map(|node| AiGnnTopologyNode {
                node_id: node.node_id,
                node_type: "operational_node".to_string(),
                attributes: BTreeMap::new(),
                labels: Vec::new(),
            })
            .collect(),
        edges: topology
            .relations
            .into_iter()
            .map(|relation| AiGnnTopologyEdge {
                from: relation.from,
                to: relation.to,
                relation: relation.rel.clone(),
                edge_type: relation.rel,
                attributes: relation.attributes,
            })
            .collect(),
    };

    let feature_schema = feature_schema();
    let node_features = context
        .operational_dgraph
        .nodes
        .iter()
        .map(|node| {
            let snapshot = context
                .node_snapshots
                .iter()
                .find(|snapshot| snapshot.node_id == node.node_id)
                .expect("validated source context node set");
            build_node_features(node.desired_services.as_slice(), snapshot)
        })
        .collect::<Vec<_>>();

    let edge_features = topology
        .edges
        .iter()
        .map(build_edge_features)
        .collect::<Vec<_>>();

    let evidence = context
        .operational_dgraph
        .nodes
        .iter()
        .map(|node| {
            let snapshot = context
                .node_snapshots
                .iter()
                .find(|snapshot| snapshot.node_id == node.node_id)
                .expect("validated source context node set");
            AiGnnNodeEvidence {
                node_id: snapshot.node_id.clone(),
                telemetry_freshness: snapshot.telemetry_freshness.clone(),
                telemetry_age_seconds: snapshot.telemetry_age_seconds,
                usable_for_inference: snapshot.usable_for_inference,
                cycle_id: snapshot.cycle_id.clone(),
                observed_at_utc: snapshot.observed_at_utc.clone(),
                snapshot_status: snapshot.snapshot_status.clone(),
                snapshot_envelope: snapshot.snapshot_envelope.clone(),
                desired_services: node
                    .desired_services
                    .iter()
                    .map(|service| AiGnnDesiredService {
                        service_id: service.service_id.clone(),
                        runtime: service.runtime.clone(),
                        desired_state: service.desired_state.clone(),
                    })
                    .collect(),
                error: snapshot.error.clone(),
            }
        })
        .collect();

    let complete = context.evidence_summary.complete;
    let eligibility = AiGnnInferenceEligibility {
        eligible: complete,
        reason: if complete {
            "complete_fresh_usable_evidence".to_string()
        } else {
            "inference_requires_complete_fresh_usable_evidence".to_string()
        },
        expected_node_count: context.evidence_summary.expected_node_count,
        usable_node_count: context.evidence_summary.usable_node_count,
        complete_evidence: complete,
    };

    Ok(AiGnnDataContract {
        schema_version: AI_GNN_DATA_CONTRACT_SCHEMA_VERSION.to_string(),
        contract_kind: AI_GNN_DATA_CONTRACT_KIND.to_string(),
        project_id: context.project_id,
        dgraph_id: context.operational_dgraph.dgraph_id.clone(),
        dgraph_revision: context.operational_dgraph.revision,
        generated_at_utc: context.generated_at_utc,
        feature_schema,
        topology,
        node_features,
        edge_features,
        evidence,
        inference_eligibility: eligibility,
        provenance: AiGnnProvenance {
            source_ai_context_schema_version: ACTIVE_AI_CONTEXT_SCHEMA_VERSION.to_string(),
            source_dgraph_acquisition_schema_version: context
                .operational_dgraph
                .schema_version
                .clone(),
            consistency_model: ACTIVE_AI_CONTEXT_CONSISTENCY.to_string(),
            complete_evidence_required_for_inference: true,
            execution_semantics: AI_GNN_EXECUTION_SEMANTICS.to_string(),
        },
        side_effects_performed: false,
    })
}

impl AiGnnDataContract {
    pub fn validate_read_only_boundary(&self) -> Result<(), String> {
        if self.schema_version != AI_GNN_DATA_CONTRACT_SCHEMA_VERSION
            || self.contract_kind != AI_GNN_DATA_CONTRACT_KIND
            || self.project_id.trim().is_empty()
            || self.dgraph_id.trim().is_empty()
            || self.dgraph_revision == 0
            || DateTime::parse_from_rfc3339(&self.generated_at_utc).is_err()
        {
            return Err("invalid AI/GNN data contract identity".to_string());
        }
        if self.side_effects_performed {
            return Err("AI/GNN data contract must be read-only".to_string());
        }
        if self.provenance.execution_semantics != AI_GNN_EXECUTION_SEMANTICS {
            return Err("AI/GNN contract must preserve proposal-only execution semantics".to_string());
        }
        if self.feature_schema.normalization.method != AI_GNN_NORMALIZATION_METHOD {
            return Err("dserver must not apply unconfigured feature normalization".to_string());
        }
        if self.inference_eligibility.eligible
            != self.inference_eligibility.complete_evidence
            || (self.inference_eligibility.eligible
                && self.inference_eligibility.usable_node_count
                    != self.inference_eligibility.expected_node_count)
        {
            return Err("AI/GNN inference eligibility is inconsistent with evidence".to_string());
        }
        Ok(())
    }
}

fn validate_source_context(context: &ActiveAiContext) -> Result<(), String> {
    if context.acquisition_mode != "active"
        || context.side_effects_performed
        || context.schema_version != ACTIVE_AI_CONTEXT_SCHEMA_VERSION
        || context.project_id != context.operational_dgraph.project_id
        || context.analysis_context.project_id != context.project_id
        || context.analysis_context.dgraph_id != context.operational_dgraph.dgraph_id
        || context.analysis_context.dgraph_revision != context.operational_dgraph.revision
        || context.analysis_context.side_effects_performed
    {
        return Err("invalid active AI context for GNN export".to_string());
    }

    validate_operational_dgraph(&context.operational_dgraph)?;

    let expected = context
        .operational_dgraph
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<HashSet<_>>();
    let actual = context
        .node_snapshots
        .iter()
        .map(|snapshot| snapshot.node_id.as_str())
        .collect::<HashSet<_>>();
    if expected != actual || actual.len() != context.node_snapshots.len() {
        return Err("AI context snapshots do not match operational D-Graph nodes".to_string());
    }
    if context.evidence_summary.expected_node_count != expected.len() as u64 {
        return Err("AI context evidence summary cardinality mismatch".to_string());
    }
    Ok(())
}

fn feature_schema() -> AiGnnFeatureSchema {
    AiGnnFeatureSchema {
        schema_version: AI_GNN_FEATURE_SCHEMA_VERSION.to_string(),
        node_numeric_features: vec![
            "desired_service_count".to_string(),
            "telemetry_age_seconds".to_string(),
            "telemetry_fresh".to_string(),
            "usable_for_inference".to_string(),
            "snapshot_entity_count".to_string(),
            "snapshot_edge_count".to_string(),
            "snapshot_event_count".to_string(),
            "snapshot_action_count".to_string(),
        ],
        node_categorical_features: vec![
            "telemetry_freshness".to_string(),
            "snapshot_status".to_string(),
        ],
        edge_numeric_features: vec!["latency_ms".to_string(), "weight".to_string()],
        edge_categorical_features: Vec::new(),
        labels: Vec::new(),
        normalization: AiGnnNormalization {
            method: AI_GNN_NORMALIZATION_METHOD.to_string(),
            parameters: BTreeMap::new(),
        },
        missing_value_policy: "null_value_with_explicit_feature_mask_no_imputation".to_string(),
        label_policy: "no_labels_assigned_by_dserver".to_string(),
    }
}

fn build_node_features(
    desired_services: &[OperationalDesiredService],
    snapshot: &AiNodeSnapshot,
) -> AiGnnNodeFeatures {
    let schema = feature_schema();
    let mut numeric = schema
        .node_numeric_features
        .iter()
        .map(|name| (name.clone(), None))
        .collect::<BTreeMap<_, _>>();
    let mut numeric_mask = schema
        .node_numeric_features
        .iter()
        .map(|name| (name.clone(), false))
        .collect::<BTreeMap<_, _>>();
    let mut categorical = schema
        .node_categorical_features
        .iter()
        .map(|name| (name.clone(), None))
        .collect::<BTreeMap<_, _>>();
    let mut categorical_mask = schema
        .node_categorical_features
        .iter()
        .map(|name| (name.clone(), false))
        .collect::<BTreeMap<_, _>>();

    set_numeric(&mut numeric, &mut numeric_mask, "desired_service_count", Some(desired_services.len() as f64));
    set_numeric(
        &mut numeric,
        &mut numeric_mask,
        "telemetry_age_seconds",
        snapshot.telemetry_age_seconds.map(|value| value as f64),
    );
    set_numeric(
        &mut numeric,
        &mut numeric_mask,
        "telemetry_fresh",
        Some((snapshot.telemetry_freshness == "fresh") as u8 as f64),
    );
    set_numeric(
        &mut numeric,
        &mut numeric_mask,
        "usable_for_inference",
        Some(snapshot.usable_for_inference as u8 as f64),
    );

    if let Some(envelope) = snapshot.snapshot_envelope.as_ref() {
        for (feature, field) in [
            ("snapshot_entity_count", "entities"),
            ("snapshot_edge_count", "edges"),
            ("snapshot_event_count", "events"),
            ("snapshot_action_count", "actions"),
        ] {
            set_numeric(
                &mut numeric,
                &mut numeric_mask,
                feature,
                snapshot_array_len(envelope, field).map(|value| value as f64),
            );
        }
    }

    set_categorical(
        &mut categorical,
        &mut categorical_mask,
        "telemetry_freshness",
        Some(snapshot.telemetry_freshness.clone()),
    );
    set_categorical(
        &mut categorical,
        &mut categorical_mask,
        "snapshot_status",
        Some(snapshot.snapshot_status.clone()),
    );

    AiGnnNodeFeatures {
        node_id: snapshot.node_id.clone(),
        numeric,
        numeric_mask,
        categorical,
        categorical_mask,
        labels: Vec::new(),
        label_mask: false,
    }
}

fn build_edge_features(edge: &AiGnnTopologyEdge) -> AiGnnEdgeFeatures {
    let mut numeric = BTreeMap::from([
        ("latency_ms".to_string(), None),
        ("weight".to_string(), None),
    ]);
    let mut numeric_mask = BTreeMap::from([
        ("latency_ms".to_string(), false),
        ("weight".to_string(), false),
    ]);
    for feature in ["latency_ms", "weight"] {
        if let Some(value) = edge.attributes.get(feature).and_then(Value::as_f64) {
            numeric.insert(feature.to_string(), Some(value));
            numeric_mask.insert(feature.to_string(), true);
        }
    }

    AiGnnEdgeFeatures {
        from: edge.from.clone(),
        to: edge.to.clone(),
        relation: edge.relation.clone(),
        numeric,
        numeric_mask,
        categorical: BTreeMap::new(),
        categorical_mask: BTreeMap::new(),
    }
}

fn set_numeric(
    values: &mut BTreeMap<String, Option<f64>>,
    masks: &mut BTreeMap<String, bool>,
    name: &str,
    value: Option<f64>,
) {
    values.insert(name.to_string(), value);
    masks.insert(name.to_string(), value.is_some());
}

fn set_categorical(
    values: &mut BTreeMap<String, Option<String>>,
    masks: &mut BTreeMap<String, bool>,
    name: &str,
    value: Option<String>,
) {
    values.insert(name.to_string(), value.clone());
    masks.insert(name.to_string(), value.is_some());
}

fn snapshot_array_len(envelope: &Value, field: &str) -> Option<usize> {
    envelope
        .get("snapshot")
        .and_then(|snapshot| snapshot.get(field))
        .and_then(Value::as_array)
        .map(Vec::len)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::core::active_ai_context::{
        ActiveAiContext, AiEvidenceSummary, AiNodeSnapshot,
    };
    use crate::core::operational_dgraph::{
        OperationalDGraph, OperationalNodeDesiredState,
    };
    use crate::core::operational_analysis_context::OperationalAnalysisContext;

    fn graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: "0.1.0".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            revision: 4,
            project_id: "project-a".to_string(),
            nodes: vec![
                OperationalNodeDesiredState {
                    node_id: "fog-01".to_string(),
                    desired_services: Vec::new(),
                },
                OperationalNodeDesiredState {
                    node_id: "cloud-01".to_string(),
                    desired_services: Vec::new(),
                },
            ],
            metadata: Some(json!({
                "relations": [{
                    "from": "fog-01",
                    "to": "cloud-01",
                    "rel": "connected_to",
                    "latency_ms": 12.5
                }]
            })),
        }
    }

    fn context(complete: bool) -> ActiveAiContext {
        let graph = graph();
        let snapshots = graph.nodes.iter().map(|node| AiNodeSnapshot {
            node_id: node.node_id.clone(),
            snapshot_status: if complete { "available" } else { "missing" }.to_string(),
            telemetry_freshness: if complete { "fresh" } else { "stale" }.to_string(),
            telemetry_age_seconds: Some(5),
            cycle_id: Some(format!("cycle-{}", node.node_id)),
            observed_at_utc: Some("2026-09-07T12:00:00Z".to_string()),
            usable_for_inference: complete,
            snapshot_envelope: Some(json!({
                "snapshot": {
                    "entities": [ {"id": "service:mqtt"} ],
                    "edges": [],
                    "events": [{"kind":"sample"}],
                    "actions": []
                }
            })),
            error: None,
        }).collect::<Vec<_>>();
        ActiveAiContext {
            schema_version: ACTIVE_AI_CONTEXT_SCHEMA_VERSION.to_string(),
            acquisition_mode: "active".to_string(),
            project_id: graph.project_id.clone(),
            generated_at_utc: "2026-09-07T12:00:00Z".to_string(),
            consistency_model: ACTIVE_AI_CONTEXT_CONSISTENCY.to_string(),
            operational_dgraph: graph,
            analysis_context: OperationalAnalysisContext {
                schema_version: "0.2.0".to_string(),
                acquisition_mode: "active".to_string(),
                project_id: "project-a".to_string(),
                dgraph_id: "dgraph:test".to_string(),
                dgraph_revision: 4,
                generated_at_utc: "2026-09-07T12:00:00Z".to_string(),
                freshness_max_age_seconds: 120,
                trigger_node_id: None,
                node_count: 2,
                side_effects_performed: false,
                nodes: Vec::new(),
            },
            node_snapshots: snapshots,
            evidence_summary: AiEvidenceSummary {
                expected_node_count: 2,
                valid_snapshot_count: if complete { 2 } else { 1 },
                usable_node_count: if complete { 2 } else { 0 },
                fresh_node_count: if complete { 2 } else { 0 },
                stale_node_count: if complete { 0 } else { 2 },
                missing_node_count: 0,
                invalid_snapshot_count: 0,
                complete,
            },
            side_effects_performed: false,
        }
    }

    #[test]
    fn complete_context_exports_typed_features_and_explicit_masks() {
        let result = build_ai_gnn_data_contract(context(true)).unwrap();
        assert_eq!(result.schema_version, "0.1.0");
        assert_eq!(result.topology.nodes[0].node_type, "operational_node");
        assert_eq!(result.topology.edges.len(), 1);
        assert_eq!(result.edge_features[0].numeric["latency_ms"], Some(12.5));
        assert_eq!(result.edge_features[0].numeric_mask["weight"], false);
        assert_eq!(result.node_features[0].numeric["snapshot_entity_count"], Some(1.0));
        assert_eq!(result.node_features[0].numeric_mask["telemetry_age_seconds"], true);
        assert_eq!(result.feature_schema.normalization.method, "none");
        assert!(result.inference_eligibility.eligible);
        result.validate_read_only_boundary().unwrap();
        let roundtrip: AiGnnDataContract =
            serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
        assert_eq!(roundtrip, result);
    }

    #[test]
    fn incomplete_context_is_exported_with_inference_masked_off() {
        let result = build_ai_gnn_data_contract(context(false)).unwrap();
        assert!(!result.inference_eligibility.eligible);
        assert_eq!(
            result.inference_eligibility.reason,
            "inference_requires_complete_fresh_usable_evidence"
        );
        assert!(!result.node_features[0].numeric_mask["usable_for_inference"]);
        result.validate_read_only_boundary().unwrap();
    }

    #[test]
    fn direct_execution_and_normalization_are_rejected() {
        let mut result = build_ai_gnn_data_contract(context(true)).unwrap();
        result.side_effects_performed = true;
        assert!(result.validate_read_only_boundary().is_err());

        let mut result = build_ai_gnn_data_contract(context(true)).unwrap();
        result.feature_schema.normalization.method = "z_score".to_string();
        assert!(result.validate_read_only_boundary().is_err());
    }
}
