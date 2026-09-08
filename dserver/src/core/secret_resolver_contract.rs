use serde::{Deserialize, Serialize};

use crate::core::artifact_reconciliation_context::ArtifactReconciliationContext;

pub const SECRET_RESOLVER_SCHEMA_VERSION: &str = "0.1.0";
pub const SECRET_RESOLVER_CONTRACT_KIND: &str = "datum_node_local_secret_resolver_contract";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolverContract {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    pub resolver: ResolverDescriptor,
    pub requirements: Vec<SecretResolutionRequirement>,
    pub execution_policy: SecretResolverExecutionPolicy,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolverDescriptor {
    pub kind: String,
    pub resolution_mode: String,
    pub values_transit_through_control_plane: bool,
    pub values_exposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolutionRequirement {
    pub service_id: String,
    pub secret_ref: String,
    pub provider: String,
    pub lookup_key: String,
    pub injection_target: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretResolverExecutionPolicy {
    pub mode: String,
    pub execution_allowed: bool,
    pub reason: String,
    pub side_effects_allowed: bool,
}

pub fn build_secret_resolver_contract(
    context: &ArtifactReconciliationContext,
) -> Result<SecretResolverContract, String> {
    crate::core::artifact_reconciliation_context::validate_artifact_reconciliation_context(context)?;

    let mut requirements = Vec::new();
    for artifact in &context.artifacts {
        for secret_ref in &artifact.secret_refs {
            let secret_ref = secret_ref.trim();
            if secret_ref.is_empty() {
                return Err(format!(
                    "service '{}' contains an empty secret reference",
                    artifact.service_id
                ));
            }
            requirements.push(SecretResolutionRequirement {
                service_id: artifact.service_id.clone(),
                secret_ref: secret_ref.to_string(),
                provider: "node_local".to_string(),
                lookup_key: secret_ref.to_string(),
                injection_target: "container_environment".to_string(),
                status: "node_local_resolution_required".to_string(),
            });
        }
    }

    let execution_allowed = requirements.is_empty();
    let reason = if execution_allowed {
        "no secret resolution is required"
    } else {
        "secret values must be resolved on the target node and injected locally; dserver never receives them"
    };

    Ok(SecretResolverContract {
        schema_version: SECRET_RESOLVER_SCHEMA_VERSION.to_string(),
        contract_kind: SECRET_RESOLVER_CONTRACT_KIND.to_string(),
        project_id: context.project_id.clone(),
        dgraph_id: context.dgraph_id.clone(),
        dgraph_revision: context.dgraph_revision,
        node_id: context.node_id.clone(),
        resolver: ResolverDescriptor {
            kind: "node_local_external_provider".to_string(),
            resolution_mode: "opaque_reference_to_node_local_material".to_string(),
            values_transit_through_control_plane: false,
            values_exposed: false,
        },
        requirements,
        execution_policy: SecretResolverExecutionPolicy {
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
    fn resolver_contract_excludes_secret_values() {
        let descriptor = ResolverDescriptor {
            kind: "node_local_external_provider".to_string(),
            resolution_mode: "opaque_reference_to_node_local_material".to_string(),
            values_transit_through_control_plane: false,
            values_exposed: false,
        };
        assert!(!descriptor.values_transit_through_control_plane);
        assert!(!descriptor.values_exposed);
    }
}
