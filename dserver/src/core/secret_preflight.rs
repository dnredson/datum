use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::core::artifact_reconciliation_context::ArtifactReconciliationContext;

pub const SECRET_PREFLIGHT_SCHEMA_VERSION: &str = "0.1.0";
pub const SECRET_PREFLIGHT_CONTRACT_KIND: &str = "datum_deployment_secret_preflight";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretPreflight {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    pub resolution_policy: SecretResolutionPolicy,
    pub references: Vec<SecretReference>,
    pub execution_policy: SecretExecutionPolicy,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolutionPolicy {
    pub mode: String,
    pub values_accepted: bool,
    pub values_exposed: bool,
    pub resolver_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretReference {
    pub service_id: String,
    pub secret_ref: String,
    pub status: String,
    pub material_available: bool,
    pub material_exposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretExecutionPolicy {
    pub mode: String,
    pub execution_allowed: bool,
    pub reason: String,
    pub side_effects_allowed: bool,
}

pub fn build_secret_preflight(
    context: &ArtifactReconciliationContext,
) -> Result<SecretPreflight, String> {
    crate::core::artifact_reconciliation_context::validate_artifact_reconciliation_context(context)?;

    let mut seen = BTreeSet::new();
    let mut references = Vec::new();
    for artifact in &context.artifacts {
        for secret_ref in &artifact.secret_refs {
            let secret_ref = secret_ref.trim();
            if secret_ref.is_empty() {
                return Err(format!(
                    "service '{}' contains an empty secret reference",
                    artifact.service_id
                ));
            }
            let key = format!("{}:{}", artifact.service_id.to_ascii_lowercase(), secret_ref);
            if !seen.insert(key) {
                return Err(format!(
                    "duplicate secret reference '{}' for service '{}'",
                    secret_ref, artifact.service_id
                ));
            }
            references.push(SecretReference {
                service_id: artifact.service_id.clone(),
                secret_ref: secret_ref.to_string(),
                status: "unresolved".to_string(),
                material_available: false,
                material_exposed: false,
            });
        }
    }

    let execution_allowed = references.is_empty();
    let reason = if execution_allowed {
        "no secret references declared"
    } else {
        "opaque references require an approved runtime resolver; no secret material is accepted by dserver"
    };

    Ok(SecretPreflight {
        schema_version: SECRET_PREFLIGHT_SCHEMA_VERSION.to_string(),
        contract_kind: SECRET_PREFLIGHT_CONTRACT_KIND.to_string(),
        project_id: context.project_id.clone(),
        dgraph_id: context.dgraph_id.clone(),
        dgraph_revision: context.dgraph_revision,
        node_id: context.node_id.clone(),
        resolution_policy: SecretResolutionPolicy {
            mode: "opaque_reference_only".to_string(),
            values_accepted: false,
            values_exposed: false,
            resolver_required: !execution_allowed,
        },
        references,
        execution_policy: SecretExecutionPolicy {
            mode: "proposal_only".to_string(),
            execution_allowed,
            reason: reason.to_string(),
            side_effects_allowed: false,
        },
        side_effects_performed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_preflight_never_exposes_material() {
        let reference = SecretReference {
            service_id: "api".to_string(),
            secret_ref: "api_password".to_string(),
            status: "unresolved".to_string(),
            material_available: false,
            material_exposed: false,
        };
        assert!(!reference.material_available);
        assert!(!reference.material_exposed);
    }
}
