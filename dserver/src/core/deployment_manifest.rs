use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEPLOYMENT_MANIFEST_VERSION: &str = "0.1.0";
const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStrategy {
    Centralized,
    Distributed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GraphDistribution {
    Full,
    NodeSlice,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentArtifactIdentity {
    pub artifact_id: String,
    pub artifact_uri: String,
    pub sha256: String,
    pub content_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeploymentRuntimeLimits {
    pub fuel: Option<u64>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeploymentAssignment {
    pub assignment_id: String,
    pub module_id: String,
    pub function_name: String,
    pub artifact: DeploymentArtifactIdentity,
    pub target_node: String,
    #[serde(default)]
    pub limits: DeploymentRuntimeLimits,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentExecutionBoundary {
    pub explicit_invocation_required: bool,
    pub automatic_execution_allowed: bool,
    pub governed_execution_authorized: bool,
}

impl Default for DeploymentExecutionBoundary {
    fn default() -> Self {
        Self {
            explicit_invocation_required: true,
            automatic_execution_allowed: false,
            governed_execution_authorized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeploymentManifest {
    pub manifest_version: String,
    pub manifest_id: String,
    pub project_id: String,
    pub strategy: DeploymentStrategy,
    pub graph_distribution: GraphDistribution,
    #[serde(default)]
    pub source_dgraph_id: Option<String>,
    pub assignments: Vec<DeploymentAssignment>,
    #[serde(default)]
    pub execution_boundary: DeploymentExecutionBoundary,
    #[serde(default)]
    pub metadata: Option<Value>,
}

pub fn build_deployment_artifact_identity(
    artifact_id: impl Into<String>,
    artifact_uri: impl Into<String>,
    sha256: &str,
) -> Result<DeploymentArtifactIdentity, String> {
    let normalized_sha256 = normalize_sha256(sha256)?;
    Ok(DeploymentArtifactIdentity {
        artifact_id: artifact_id.into(),
        artifact_uri: artifact_uri.into(),
        content_id: format!("sha256:{normalized_sha256}"),
        sha256: normalized_sha256,
    })
}

pub fn validate_deployment_manifest(manifest: &DeploymentManifest) -> Result<(), String> {
    if manifest.manifest_version.trim() != DEPLOYMENT_MANIFEST_VERSION {
        return Err(format!(
            "manifest_version must be {DEPLOYMENT_MANIFEST_VERSION}"
        ));
    }
    if manifest.manifest_id.trim().is_empty() {
        return Err("manifest_id must not be empty".to_string());
    }
    if manifest.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }
    if let Some(source_dgraph_id) = manifest.source_dgraph_id.as_deref() {
        if source_dgraph_id.trim().is_empty() {
            return Err("source_dgraph_id must not be empty when present".to_string());
        }
    }

    validate_strategy_distribution_pair(&manifest.strategy, &manifest.graph_distribution)?;
    validate_execution_boundary(&manifest.execution_boundary)?;

    if manifest.assignments.is_empty() {
        return Err("deployment manifest must contain at least one assignment".to_string());
    }

    let mut seen_assignment_ids = HashSet::new();
    for assignment in &manifest.assignments {
        validate_assignment(assignment)?;
        if !seen_assignment_ids.insert(assignment.assignment_id.trim()) {
            return Err(format!(
                "duplicate assignment_id is not allowed: {}",
                assignment.assignment_id
            ));
        }
    }

    if manifest.strategy == DeploymentStrategy::Centralized {
        let central_target = manifest.assignments[0].target_node.trim();
        if manifest
            .assignments
            .iter()
            .any(|assignment| assignment.target_node.trim() != central_target)
        {
            return Err(
                "centralized/full deployment requires every assignment to target the same node"
                    .to_string(),
            );
        }
    }

    Ok(())
}

fn validate_strategy_distribution_pair(
    strategy: &DeploymentStrategy,
    graph_distribution: &GraphDistribution,
) -> Result<(), String> {
    match (strategy, graph_distribution) {
        (DeploymentStrategy::Centralized, GraphDistribution::Full)
        | (DeploymentStrategy::Distributed, GraphDistribution::NodeSlice) => Ok(()),
        (DeploymentStrategy::Centralized, GraphDistribution::NodeSlice) => Err(
            "centralized strategy requires graph_distribution=full".to_string(),
        ),
        (DeploymentStrategy::Distributed, GraphDistribution::Full) => Err(
            "distributed strategy requires graph_distribution=node_slice".to_string(),
        ),
    }
}

fn validate_execution_boundary(boundary: &DeploymentExecutionBoundary) -> Result<(), String> {
    if !boundary.explicit_invocation_required {
        return Err("explicit_invocation_required must remain true".to_string());
    }
    if boundary.automatic_execution_allowed {
        return Err("automatic_execution_allowed must remain false".to_string());
    }
    if boundary.governed_execution_authorized {
        return Err("governed_execution_authorized must remain false".to_string());
    }
    Ok(())
}

fn validate_assignment(assignment: &DeploymentAssignment) -> Result<(), String> {
    if assignment.assignment_id.trim().is_empty() {
        return Err("assignment_id must not be empty".to_string());
    }
    if assignment.module_id.trim().is_empty() {
        return Err("module_id must not be empty".to_string());
    }
    if assignment.function_name.trim().is_empty() {
        return Err("function_name must not be empty".to_string());
    }
    if assignment.target_node.trim().is_empty() {
        return Err("target_node must not be empty".to_string());
    }
    if assignment.artifact.artifact_id.trim().is_empty() {
        return Err("artifact_id must not be empty".to_string());
    }
    if assignment.artifact.artifact_uri.trim().is_empty() {
        return Err("artifact_uri must not be empty".to_string());
    }

    let normalized_sha256 = normalize_sha256(&assignment.artifact.sha256)?;
    let expected_content_id = format!("sha256:{normalized_sha256}");
    if assignment.artifact.content_id.trim().to_ascii_lowercase() != expected_content_id {
        return Err(format!(
            "artifact content_id must match sha256 digest: expected {expected_content_id}"
        ));
    }

    if assignment.limits.fuel == Some(0) {
        return Err("assignment fuel must be positive when present".to_string());
    }
    if assignment.limits.timeout_ms == Some(0) {
        return Err("assignment timeout_ms must be positive when present".to_string());
    }

    Ok(())
}

fn normalize_sha256(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let digest = trimmed.strip_prefix("sha256:").unwrap_or(trimmed);
    if digest.len() != SHA256_HEX_LEN || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            "artifact sha256 must contain exactly 64 hexadecimal characters, optionally prefixed with 'sha256:'"
                .to_string(),
        );
    }
    Ok(digest.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TEST_SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn assignment(id: &str, node: &str) -> DeploymentAssignment {
        DeploymentAssignment {
            assignment_id: id.to_string(),
            module_id: format!("module-{id}"),
            function_name: "run".to_string(),
            artifact: build_deployment_artifact_identity(
                format!("artifact-{id}"),
                format!("file:///artifacts/{id}.wasm"),
                TEST_SHA,
            )
            .expect("test digest should be valid"),
            target_node: node.to_string(),
            limits: DeploymentRuntimeLimits {
                fuel: Some(1_000_000),
                timeout_ms: Some(1_000),
            },
            metadata: None,
        }
    }

    fn centralized_manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest:centralized:1".to_string(),
            project_id: "smart-farm".to_string(),
            strategy: DeploymentStrategy::Centralized,
            graph_distribution: GraphDistribution::Full,
            source_dgraph_id: Some("dgraph:snapshot:1".to_string()),
            assignments: vec![assignment("sensor-monitor", "fog-01")],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    fn distributed_manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest:distributed:1".to_string(),
            project_id: "smart-farm".to_string(),
            strategy: DeploymentStrategy::Distributed,
            graph_distribution: GraphDistribution::NodeSlice,
            source_dgraph_id: Some("dgraph:snapshot:1".to_string()),
            assignments: vec![
                assignment("sensor-monitor", "fog-01"),
                assignment("mqtt-adapter", "fog-02"),
            ],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: Some(json!({"reason": "c63-test"})),
        }
    }

    #[test]
    fn centralized_full_manifest_is_valid() {
        assert!(validate_deployment_manifest(&centralized_manifest()).is_ok());
    }

    #[test]
    fn distributed_node_slice_manifest_is_valid() {
        assert!(validate_deployment_manifest(&distributed_manifest()).is_ok());
    }

    #[test]
    fn centralized_node_slice_pair_is_rejected() {
        let mut manifest = centralized_manifest();
        manifest.graph_distribution = GraphDistribution::NodeSlice;
        let error = validate_deployment_manifest(&manifest).expect_err("invalid pair must fail");
        assert!(error.contains("centralized strategy requires"));
    }

    #[test]
    fn distributed_full_pair_is_rejected() {
        let mut manifest = distributed_manifest();
        manifest.graph_distribution = GraphDistribution::Full;
        let error = validate_deployment_manifest(&manifest).expect_err("invalid pair must fail");
        assert!(error.contains("distributed strategy requires"));
    }

    #[test]
    fn centralized_manifest_requires_one_target_node() {
        let mut manifest = centralized_manifest();
        manifest
            .assignments
            .push(assignment("mqtt-adapter", "fog-02"));
        let error = validate_deployment_manifest(&manifest)
            .expect_err("multiple centralized targets must fail");
        assert!(error.contains("same node"));
    }

    #[test]
    fn duplicate_assignment_id_is_rejected() {
        let mut manifest = distributed_manifest();
        manifest.assignments[1].assignment_id = manifest.assignments[0].assignment_id.clone();
        let error = validate_deployment_manifest(&manifest)
            .expect_err("duplicate assignment id must fail");
        assert!(error.contains("duplicate assignment_id"));
    }

    #[test]
    fn malformed_sha256_is_rejected() {
        let mut manifest = centralized_manifest();
        manifest.assignments[0].artifact.sha256 = "not-a-digest".to_string();
        let error = validate_deployment_manifest(&manifest).expect_err("bad digest must fail");
        assert!(error.contains("64 hexadecimal characters"));
    }

    #[test]
    fn content_id_must_match_sha256() {
        let mut manifest = centralized_manifest();
        manifest.assignments[0].artifact.content_id = format!("sha256:{}", "f".repeat(64));
        let error = validate_deployment_manifest(&manifest)
            .expect_err("content identity mismatch must fail");
        assert!(error.contains("content_id must match"));
    }

    #[test]
    fn artifact_builder_normalizes_prefixed_uppercase_sha256() {
        let identity = build_deployment_artifact_identity(
            "artifact-1",
            "file:///artifact.wasm",
            &format!("sha256:{}", TEST_SHA.to_ascii_uppercase()),
        )
        .expect("digest should normalize");
        assert_eq!(identity.sha256, TEST_SHA);
        assert_eq!(identity.content_id, format!("sha256:{TEST_SHA}"));
    }

    #[test]
    fn zero_fuel_is_rejected() {
        let mut manifest = centralized_manifest();
        manifest.assignments[0].limits.fuel = Some(0);
        let error = validate_deployment_manifest(&manifest).expect_err("zero fuel must fail");
        assert!(error.contains("fuel must be positive"));
    }

    #[test]
    fn zero_timeout_is_rejected() {
        let mut manifest = centralized_manifest();
        manifest.assignments[0].limits.timeout_ms = Some(0);
        let error = validate_deployment_manifest(&manifest).expect_err("zero timeout must fail");
        assert!(error.contains("timeout_ms must be positive"));
    }

    #[test]
    fn explicit_invocation_must_remain_required() {
        let mut manifest = centralized_manifest();
        manifest.execution_boundary.explicit_invocation_required = false;
        let error = validate_deployment_manifest(&manifest)
            .expect_err("implicit invocation must remain blocked");
        assert!(error.contains("explicit_invocation_required"));
    }

    #[test]
    fn automatic_execution_must_remain_disabled() {
        let mut manifest = centralized_manifest();
        manifest.execution_boundary.automatic_execution_allowed = true;
        let error = validate_deployment_manifest(&manifest)
            .expect_err("automatic execution must remain blocked");
        assert!(error.contains("automatic_execution_allowed"));
    }

    #[test]
    fn governed_execution_authorization_cannot_be_claimed() {
        let mut manifest = centralized_manifest();
        manifest.execution_boundary.governed_execution_authorized = true;
        let error = validate_deployment_manifest(&manifest)
            .expect_err("manifest cannot grant governed authorization");
        assert!(error.contains("governed_execution_authorized"));
    }

    #[test]
    fn empty_assignment_list_is_rejected() {
        let mut manifest = centralized_manifest();
        manifest.assignments.clear();
        let error = validate_deployment_manifest(&manifest)
            .expect_err("empty manifest assignments must fail");
        assert!(error.contains("at least one assignment"));
    }

    #[test]
    fn serde_uses_reserved_strategy_and_distribution_strings() {
        let manifest = distributed_manifest();
        let value = serde_json::to_value(&manifest).expect("manifest should serialize");
        assert_eq!(value["strategy"].as_str(), Some("distributed"));
        assert_eq!(value["graph_distribution"].as_str(), Some("node_slice"));

        let round_trip: DeploymentManifest =
            serde_json::from_value(value).expect("manifest should deserialize");
        assert_eq!(round_trip, manifest);
    }
}
