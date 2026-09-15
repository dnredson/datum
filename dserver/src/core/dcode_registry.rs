//! Server-owned registry of canonical `datum.dserv-artifact/1` descriptors
//! (ADR-0021). DServer stores and verifies canonical METADATA and digest
//! IDENTITY only -- the module BYTES themselves live in the D-Node-local
//! content-addressed module store (DATUM `dcode::module_store`, operational,
//! never persisted here). This registry is a separate domain/type from
//! `ServiceArtifact` (ADR-0005): it never gains container/runtime fields,
//! and `ServiceArtifact` is never treated as if it were D-Code.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::dserv_artifact::CanonicalDServArtifactV1;
use crate::storage::control_state::{load_domain_json, persist_domain_json, DOMAIN_DCODE_REGISTRY};

pub const DCODE_REGISTRY_STATE_VERSION: &str = "dcode_artifact_registry_v1";

fn length_prefixed_component(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

/// Keyed by `(application_id, dserv_id)` -- the D-Serv artifact fixture
/// carries no `project_id` (ADR-0009: the D-Graph/D-Serv plane never does),
/// so this is the smallest key that actually identifies "the D-Code for
/// this D-Serv", exactly mirroring `ddeploy::application_key`'s
/// length-prefixed collision-free composition.
pub fn dserv_artifact_key(application_id: &str, dserv_id: &str) -> String {
    format!("{}{}", length_prefixed_component(application_id), length_prefixed_component(dserv_id))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DCodeArtifactRegistryStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub artifacts: BTreeMap<String, CanonicalDServArtifactV1>,
}

impl DCodeArtifactRegistryStateV1 {
    pub fn empty() -> Self {
        Self { state_version: DCODE_REGISTRY_STATE_VERSION.to_string(), artifacts: BTreeMap::new() }
    }

    pub fn get(&self, application_id: &str, dserv_id: &str) -> Option<&CanonicalDServArtifactV1> {
        self.artifacts.get(&dserv_artifact_key(application_id, dserv_id))
    }
}

#[derive(Debug)]
pub enum DeclareDServArtifactError {
    Invalid(Vec<crate::core::canonical_dgraph::CanonicalValidationFinding>),
    Conflict(String),
}

/// Idempotent-but-conflict-detecting declaration, mirroring
/// `dnode_registry`'s precedent exactly: re-declaring byte-identical
/// content is a no-op; declaring DIFFERENT content for an existing
/// `(application_id, dserv_id)` fails closed. The registry key is
/// `(application_id, dserv_id)` -- NOT `artifact_id`/`version` -- so a
/// caller cannot "declare a real content change" merely by changing
/// `artifact_id`/`version` while the `(application_id, dserv_id)` pair
/// stays the same; ANY different descriptor at that key conflicts,
/// regardless of what its `artifact_id`/`version` say (independent review
/// v2, R8: an earlier version of this comment incorrectly implied
/// `artifact_id`/`version` was itself an escape hatch). v0.1 has no
/// artifact replacement/version-transition mechanism at all -- one
/// immutable registered canonical artifact per `(application_id,
/// dserv_id)`, full stop; redeployment/migration to a new artifact is an
/// explicitly future concern, not implemented here.
pub fn declare_dserv_artifact(
    state: &mut DCodeArtifactRegistryStateV1,
    artifact: CanonicalDServArtifactV1,
) -> Result<(), DeclareDServArtifactError> {
    let findings = artifact.validate();
    if !findings.is_empty() {
        return Err(DeclareDServArtifactError::Invalid(findings));
    }

    let key = dserv_artifact_key(&artifact.application_id, &artifact.dserv_id);
    if let Some(existing) = state.artifacts.get(&key) {
        if normalized(existing) == normalized(&artifact) {
            return Ok(());
        }
        return Err(DeclareDServArtifactError::Conflict(format!(
            "dserv artifact for application '{}' service '{}' is already declared with different content; one immutable artifact is registered per (application_id, dserv_id) in v0.1 -- artifact replacement/version transition is not implemented",
            artifact.application_id, artifact.dserv_id
        )));
    }

    state.artifacts.insert(key, artifact);
    Ok(())
}

fn normalized(artifact: &CanonicalDServArtifactV1) -> CanonicalDServArtifactV1 {
    let mut sorted = artifact.clone();
    sorted.abi.required_exports.sort();
    sorted.abi.allowed_imports.sort();
    sorted
}

/// Fail-closed relational integrity check, invoked from `load_state`.
/// Mirrors `dnode_registry::validate_persisted_dnode_registry_integrity`'s
/// shape: map-key consistency, each stored artifact independently
/// re-validated, no silent normalization/repair of persisted corruption.
pub fn validate_persisted_dcode_registry_integrity(state: &DCodeArtifactRegistryStateV1) -> Result<(), String> {
    if state.state_version != DCODE_REGISTRY_STATE_VERSION {
        return Err(format!(
            "unsupported dcode artifact registry state_version '{}', expected '{DCODE_REGISTRY_STATE_VERSION}'",
            state.state_version
        ));
    }

    for (key, artifact) in &state.artifacts {
        let expected_key = dserv_artifact_key(&artifact.application_id, &artifact.dserv_id);
        if *key != expected_key {
            return Err(format!(
                "dcode artifact stored under key '{key}' does not match dserv_artifact_key('{}', '{}') = '{expected_key}'",
                artifact.application_id, artifact.dserv_id
            ));
        }
        let findings = artifact.validate();
        if !findings.is_empty() {
            return Err(format!(
                "dcode artifact '{key}' (application_id='{}', dserv_id='{}') no longer validates cleanly: {:?}",
                artifact.application_id, artifact.dserv_id, findings
            ));
        }
    }

    Ok(())
}

pub fn load_state(db_path: &std::path::Path) -> Result<DCodeArtifactRegistryStateV1, String> {
    match load_domain_json::<DCodeArtifactRegistryStateV1>(db_path, DOMAIN_DCODE_REGISTRY)? {
        Some(state) => {
            validate_persisted_dcode_registry_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DCodeArtifactRegistryStateV1::empty()),
    }
}

pub fn persist_state(db_path: &std::path::Path, state: &DCodeArtifactRegistryStateV1) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DCODE_REGISTRY, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dserv_artifact::{
        CanonicalDServArtifactV1, DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1,
        DServArtifactLimitsV1, DServArtifactPortsV1, DServArtifactSafetyV1, CANONICAL_DSERV_ARTIFACT_SCHEMA,
        DNODE_ABI_V0_REQUIRED_EXPORTS,
    };
    use serde_json::Value;

    fn artifact(application_id: &str, dserv_id: &str, version: &str) -> CanonicalDServArtifactV1 {
        CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: format!("dserv:{application_id}:{dserv_id}"),
            version: version.to_string(),
            application_id: application_id.to_string(),
            dserv_id: dserv_id.to_string(),
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{}", "a".repeat(64)),
                sha256: "a".repeat(64),
                content_id: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 64,
            },
            abi: DServArtifactAbiV1 {
                name: "datum-dnode".to_string(),
                version: "0".to_string(),
                required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
                allowed_imports: Vec::new(),
            },
            ports: DServArtifactPortsV1::default(),
            limits: DServArtifactLimitsV1 {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 100,
                max_memory_pages: 4,
                max_message_bytes: 4096,
                max_emits_per_invocation: 1,
            },
            safety: DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: DServArtifactConstraintsV1::default(),
            qos: Value::Null,
            provenance: Value::Null,
            probes: Value::Null,
            signature_ref: None,
        }
    }

    #[test]
    fn declaring_a_valid_artifact_succeeds() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        assert!(state.get("app:a", "svc").is_some());
    }

    #[test]
    fn redeclaring_identical_content_is_idempotent() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        declare_dserv_artifact(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        assert_eq!(state.artifacts.len(), 1);
    }

    #[test]
    fn redeclaring_different_content_fails_closed() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        let mut different = artifact("app:a", "svc", "0.1.0");
        different.dcode.sha256 = "b".repeat(64);
        different.dcode.content_id = format!("sha256:{}", "b".repeat(64));
        different.dcode.uri = format!("datum-blob://sha256/{}", "b".repeat(64)); // keep F7A's uri/sha256/content_id binding consistent
        let error = declare_dserv_artifact(&mut state, different).unwrap_err();
        assert!(matches!(error, DeclareDServArtifactError::Conflict(_)));
    }

    #[test]
    fn invalid_artifact_is_rejected_before_insertion() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        let mut bad = artifact("app:a", "svc", "0.1.0");
        bad.limits.fuel_per_invocation = 0;
        let error = declare_dserv_artifact(&mut state, bad).unwrap_err();
        assert!(matches!(error, DeclareDServArtifactError::Invalid(_)));
        assert!(state.get("app:a", "svc").is_none());
    }

    #[test]
    fn persisted_map_key_mismatch_fails_closed_on_reload() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        state.artifacts.insert("wrong-key".to_string(), artifact("app:a", "svc", "0.1.0"));
        let error = validate_persisted_dcode_registry_integrity(&state).unwrap_err();
        assert!(error.contains("does not match dserv_artifact_key"));
    }

    #[test]
    fn persisted_invalid_artifact_fails_closed_on_reload() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        let mut bad = artifact("app:a", "svc", "0.1.0");
        bad.abi.allowed_imports.push("evil".to_string());
        state.artifacts.insert(dserv_artifact_key("app:a", "svc"), bad);
        let error = validate_persisted_dcode_registry_integrity(&state).unwrap_err();
        assert!(error.contains("no longer validates cleanly"));
    }

    #[test]
    fn unsupported_state_version_fails_closed_on_reload() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        state.state_version = "bogus".to_string();
        let error = validate_persisted_dcode_registry_integrity(&state).unwrap_err();
        assert!(error.contains("unsupported dcode artifact registry state_version"));
    }

    #[test]
    fn valid_multi_artifact_registry_passes_integrity_check() {
        let mut state = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut state, artifact("app:a", "decode", "0.1.0")).unwrap();
        declare_dserv_artifact(&mut state, artifact("app:a", "alert", "0.1.0")).unwrap();
        assert!(validate_persisted_dcode_registry_integrity(&state).is_ok());
    }
}
