use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AI_GNN_DATA_CONTRACT_SCHEMA_VERSION: &str = "0.1.0";
pub const AI_GNN_DATA_CONTRACT_KIND: &str = "datum_ai_gnn_input";

/// Transport-neutral contract for external AI/GNN consumers.
///
/// This contract is descriptive only. It carries topology, desired/observed
/// evidence and provenance, but intentionally carries no executable host action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnDataContract {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub generated_at_utc: String,
    pub topology: AiGnnTopology,
    pub evidence: Vec<AiGnnNodeEvidence>,
    pub provenance: AiGnnProvenance,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnTopology {
    pub nodes: Vec<AiGnnTopologyNode>,
    pub edges: Vec<AiGnnTopologyEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnTopologyNode {
    pub node_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnTopologyEdge {
    pub from: String,
    pub to: String,
    pub relation: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiGnnNodeEvidence {
    pub node_id: String,
    pub telemetry_freshness: String,
    pub telemetry_age_seconds: Option<u64>,
    pub usable_for_inference: bool,
    pub cycle_id: Option<String>,
    pub observed_at_utc: Option<String>,
    /// Lossless snapshot payload from C67 when available.
    pub snapshot_envelope: Option<Value>,
    /// Desired state remains separate from observed evidence so model features
    /// cannot silently collapse policy intent into runtime observation.
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
pub struct AiGnnProvenance {
    pub source_ai_context_schema_version: String,
    pub source_dgraph_acquisition_schema_version: String,
    pub consistency_model: String,
    pub complete_evidence_required_for_inference: bool,
    pub execution_semantics: String,
}

impl AiGnnDataContract {
    pub fn validate_read_only_boundary(&self) -> Result<(), String> {
        if self.schema_version.trim().is_empty()
            || self.contract_kind != AI_GNN_DATA_CONTRACT_KIND
            || self.project_id.trim().is_empty()
            || self.dgraph_id.trim().is_empty()
            || self.generated_at_utc.trim().is_empty()
        {
            return Err("invalid AI/GNN data contract identity".to_string());
        }
        if self.side_effects_performed {
            return Err("AI/GNN data contract must be read-only".to_string());
        }
        if self.provenance.execution_semantics != "proposal_only_no_direct_execution" {
            return Err("AI/GNN contract must preserve proposal-only execution semantics".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_rejects_direct_execution_semantics() {
        let contract = AiGnnDataContract {
            schema_version: AI_GNN_DATA_CONTRACT_SCHEMA_VERSION.to_string(),
            contract_kind: AI_GNN_DATA_CONTRACT_KIND.to_string(),
            project_id: "monitor-f49c8".to_string(),
            dgraph_id: "dgraph:o2-mqtt-role-1".to_string(),
            dgraph_revision: 4,
            generated_at_utc: "2026-09-07T16:00:00Z".to_string(),
            topology: AiGnnTopology { nodes: vec![], edges: vec![] },
            evidence: vec![],
            provenance: AiGnnProvenance {
                source_ai_context_schema_version: "0.1.0".to_string(),
                source_dgraph_acquisition_schema_version: "0.1.0".to_string(),
                consistency_model: "single_accepted_dgraph_revision_non_atomic_latest_per_node".to_string(),
                complete_evidence_required_for_inference: true,
                execution_semantics: "direct_execution".to_string(),
            },
            side_effects_performed: false,
        };
        assert!(contract.validate_read_only_boundary().is_err());
    }
}
