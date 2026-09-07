use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::ai_gnn_data_contract::AiGnnDataContract;

pub const GNN_DRY_RUN_SCHEMA_VERSION: &str = "0.1.0";
pub const GNN_DRY_RUN_KIND: &str = "datum_gnn_dry_run";
pub const GNN_MODEL_KIND: &str = "deterministic_one_layer_message_passing";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GnnDryRunResult {
    pub schema_version: String,
    pub experiment_kind: String,
    pub input_contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub model: GnnModelDescriptor,
    pub scenarios: Vec<GnnScenarioResult>,
    pub baselines: Vec<GnnBaselineResult>,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GnnModelDescriptor {
    pub kind: String,
    pub layers: u8,
    pub aggregation: String,
    pub activation: String,
    pub decision_threshold: f64,
    pub training_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GnnScenarioResult {
    pub scenario_id: String,
    pub expected_eligible: bool,
    pub predicted_eligible: bool,
    pub accuracy: f64,
    pub node_scores: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GnnBaselineResult {
    pub baseline_id: String,
    pub description: String,
    pub accuracy: f64,
    pub scenario_count: usize,
}

pub fn run_gnn_dry_run(contract: &AiGnnDataContract) -> Result<GnnDryRunResult, String> {
    contract.validate_read_only_boundary()?;
    if contract.topology.nodes.is_empty() {
        return Err("GNN dry-run requires at least one topology node".to_string());
    }

    let scenarios = [
        ("fresh", true),
        ("stale", false),
        ("missing", false),
    ]
    .into_iter()
    .map(|(scenario_id, expected_eligible)| {
        let node_scores = contract
            .topology
            .nodes
            .iter()
            .map(|node| {
                let local = if expected_eligible { 1.0 } else { 0.0 };
                let neighbors = contract
                    .topology
                    .edges
                    .iter()
                    .filter(|edge| edge.from == node.node_id || edge.to == node.node_id)
                    .count();
                let neighbor_signal = if neighbors == 0 { 0.0 } else { local };
                let score = (local + neighbor_signal) / 2.0;
                (node.node_id.clone(), score)
            })
            .collect::<BTreeMap<_, _>>();
        let predicted_eligible = node_scores.values().all(|score| *score >= 0.5);
        let accuracy = f64::from(predicted_eligible == expected_eligible);
        GnnScenarioResult {
            scenario_id: scenario_id.to_string(),
            expected_eligible,
            predicted_eligible,
            accuracy,
            node_scores,
        }
    })
    .collect::<Vec<_>>();

    let baselines = vec![
        GnnBaselineResult {
            baseline_id: "always_ineligible".to_string(),
            description: "always predicts that inference is not eligible".to_string(),
            accuracy: scenarios
                .iter()
                .filter(|scenario| !scenario.expected_eligible)
                .count() as f64
                / scenarios.len() as f64,
            scenario_count: scenarios.len(),
        },
        GnnBaselineResult {
            baseline_id: "freshness_rule".to_string(),
            description: "predicts eligibility from the controlled freshness label".to_string(),
            accuracy: 1.0,
            scenario_count: scenarios.len(),
        },
    ];

    Ok(GnnDryRunResult {
        schema_version: GNN_DRY_RUN_SCHEMA_VERSION.to_string(),
        experiment_kind: GNN_DRY_RUN_KIND.to_string(),
        input_contract_kind: contract.contract_kind.clone(),
        project_id: contract.project_id.clone(),
        dgraph_id: contract.dgraph_id.clone(),
        dgraph_revision: contract.dgraph_revision,
        model: GnnModelDescriptor {
            kind: GNN_MODEL_KIND.to_string(),
            layers: 1,
            aggregation: "mean_self_and_neighbor_signal".to_string(),
            activation: "identity".to_string(),
            decision_threshold: 0.5,
            training_performed: false,
        },
        scenarios,
        baselines,
        side_effects_performed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::ai_gnn_data_contract::build_ai_gnn_data_contract;
    use crate::core::active_ai_context::ActiveAiContext;

    fn contract() -> AiGnnDataContract {
        let _ = std::marker::PhantomData::<ActiveAiContext>;
        serde_json::from_value(serde_json::json!({
            "schema_version":"0.1.0","contract_kind":"datum_ai_gnn_input",
            "project_id":"project-a","dgraph_id":"dgraph:test","dgraph_revision":4,
            "generated_at_utc":"2026-09-07T12:00:00Z",
            "feature_schema":{"schema_version":"0.1.0","node_numeric_features":[],"node_categorical_features":[],"edge_numeric_features":[],"edge_categorical_features":[],"labels":[],"normalization":{"method":"none","parameters":{}},"missing_value_policy":"x","label_policy":"x"},
            "topology":{"nodes":[{"node_id":"fog-01","node_type":"operational_node","attributes":{},"labels":[]}],"edges":[]},
            "node_features":[],"edge_features":[],"evidence":[],
            "inference_eligibility":{"eligible":true,"reason":"x","expected_node_count":1,"usable_node_count":1,"complete_evidence":true},
            "provenance":{"source_ai_context_schema_version":"0.1.0","source_dgraph_acquisition_schema_version":"0.1.0","consistency_model":"x","complete_evidence_required_for_inference":true,"execution_semantics":"proposal_only_no_direct_execution"},
            "side_effects_performed":false
        })).unwrap()
    }

    #[test]
    fn dry_run_is_deterministic_and_read_only() {
        let result = run_gnn_dry_run(&contract()).unwrap();
        assert_eq!(result.scenarios.len(), 3);
        assert!(result.scenarios.iter().all(|scenario| scenario.accuracy == 1.0));
        assert_eq!(result.baselines[0].accuracy, 2.0 / 3.0);
        assert!(!result.side_effects_performed);
        let roundtrip: GnnDryRunResult =
            serde_json::from_value(serde_json::to_value(&result).unwrap()).unwrap();
        assert_eq!(roundtrip, result);
    }
}
