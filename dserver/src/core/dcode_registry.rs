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

/// Legacy v1 load path -- kept only so `migrate_v1_state_to_v2` and its own
/// tests can exercise the exact pre-117G on-disk shape. NOT used by any
/// governed production path after Phase 117G (see `load_state_v2`).
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

// =========================================================================
// Phase 117G: versioned, immutable-per-revision registry.
//
// The v1 one-slot semantic (`(application_id, dserv_id) -> single
// descriptor`) conflated logical D-Serv identity with exact executable
// revision (Phase 117D/117F). v2 separates them: `(application_id,
// dserv_id)` still identifies the LOGICAL D-Serv (unchanged), but the
// registry may now hold MULTIPLE immutable descriptor revisions for that
// same logical D-Serv, keyed additionally by each revision's own
// `descriptor_digest` (`recompute_dserv_artifact_digest`, never a
// caller-supplied value). Declaring a new revision NEVER replaces or
// removes an existing one, and -- load-bearing, Phase 117F's central
// security invariant -- declaring a new revision alone changes NOTHING
// about what is currently active for that D-Serv. Activeness is exclusively
// a DDeploy relation (`ddeploy`/`ddeploy_v2`'s own `dcode_artifact_authority`
// binding), never a registry concept; this module intentionally has no
// "latest"/"current" selector anywhere.
//
// `project_id` is deliberately NOT part of this key. `CanonicalDServArtifactV1`
// itself carries no `project_id` (mirroring `CanonicalDGraphV1`'s own
// project-less design, ADR-0009), and ADR-0009 explicitly intends the SAME
// `application_id` to be legitimately reused/deployed across multiple
// projects. Once the key includes `descriptor_digest`, two projects
// deploying the identical content share one immutable, content-addressed
// entry (ordinary, safe deduplication); two projects deploying genuinely
// different content for the "same" logical D-Serv get two different
// digests and therefore two different, non-colliding entries. Project
// scoping belongs entirely to DDeploy's own (already project-scoped)
// proposal/acceptance binding, which names WHICH exact digest a given
// project's deployment uses -- never to this canonical, project-less
// content registry. Adding `project_id` here would duplicate installation
// scope into canonical artifact identity for no descriptor-digest-versioning
// benefit that isn't already provided.
// =========================================================================

pub const DCODE_REGISTRY_STATE_VERSION_V2: &str = "dcode_artifact_registry_v2";

/// The minimal, exact D-Code executable-revision authority binding
/// (Phase 117G): `descriptor_digest` alone, because it is the exact
/// revision identity (117D/117F/117E) and already transitively commits to
/// module content, `artifact_id`/`version` (non-authoritative, 117D), ABI,
/// safety, limits, ports, and service binding -- everything in the
/// descriptor. No redundant `artifact_id`/`version`/module-digest field is
/// carried here: they would either duplicate what `descriptor_digest`
/// already commits to, or (for `artifact_id`/`version`) add fields Phase
/// 117D already proved carry no authority. Shared verbatim by both
/// `ddeploy::DDeployProposalV1` and `ddeploy_v2::DDeployProposalV2` (Phase
/// 117G Section 14: one shared binding type, not two divergent ones).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCodeArtifactAuthorityBindingV1 {
    pub descriptor_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DCodeArtifactRegistryStateV2 {
    pub state_version: String,
    /// Outer key: `dserv_artifact_key(application_id, dserv_id)` (the same
    /// logical-D-Serv key v1 used exclusively). Inner key: the revision's
    /// own `descriptor_digest`. Multiple inner entries are exactly multiple
    /// coexisting immutable revisions of the same logical D-Serv.
    #[serde(default)]
    pub artifacts: BTreeMap<String, BTreeMap<String, CanonicalDServArtifactV1>>,
}

impl DCodeArtifactRegistryStateV2 {
    pub fn empty() -> Self {
        Self { state_version: DCODE_REGISTRY_STATE_VERSION_V2.to_string(), artifacts: BTreeMap::new() }
    }

    /// The ONLY lookup a governed/authorized path may ever use. Never
    /// ambiguous by construction: a caller who does not know the exact
    /// `descriptor_digest` they intend simply gets `None`, never a guess.
    pub fn get_exact(&self, application_id: &str, dserv_id: &str, descriptor_digest: &str) -> Option<&CanonicalDServArtifactV1> {
        self.artifacts.get(&dserv_artifact_key(application_id, dserv_id))?.get(descriptor_digest)
    }

    /// Compatibility/debugging-only lookup by logical identity alone.
    /// Returns `Ok(None)` if no revision is registered, `Ok(Some(_))` only
    /// when EXACTLY one revision exists, and a stable
    /// `dcode_artifact_lookup_ambiguous`-prefixed error the moment more than
    /// one revision exists -- it never guesses "latest"/"newest"/lexical
    /// order. Must never be used by any governed invocation-authorization
    /// path (see `dcode_authorization`'s exact-binding lookup instead).
    pub fn get_logical_if_unambiguous(&self, application_id: &str, dserv_id: &str) -> Result<Option<&CanonicalDServArtifactV1>, String> {
        match self.artifacts.get(&dserv_artifact_key(application_id, dserv_id)) {
            None => Ok(None),
            Some(revisions) if revisions.len() == 1 => Ok(revisions.values().next()),
            Some(revisions) if revisions.is_empty() => Ok(None),
            Some(revisions) => Err(format!(
                "dcode_artifact_lookup_ambiguous: {} revisions are registered for application_id='{application_id}' dserv_id='{dserv_id}'; an exact descriptor_digest is required",
                revisions.len()
            )),
        }
    }

    pub fn revision_count(&self, application_id: &str, dserv_id: &str) -> usize {
        self.artifacts.get(&dserv_artifact_key(application_id, dserv_id)).map(BTreeMap::len).unwrap_or(0)
    }
}

#[derive(Debug)]
pub enum DeclareDServArtifactRevisionError {
    Invalid(Vec<crate::core::canonical_dgraph::CanonicalValidationFinding>),
    /// Structurally should be unreachable (the inner key IS the artifact's
    /// own content digest, so two entries under the same digest key are
    /// identical by construction barring a SHA-256 collision), but modeled
    /// honestly rather than asserted away, mirroring this project's
    /// established precedent (`DCompileLoweringError::DigestComputationFailed`
    /// and siblings) for theoretically-unreachable-but-honestly-modeled cases.
    Conflict(String),
    DigestComputationFailed(String),
}

/// Declares one immutable descriptor revision. Never replaces or removes
/// any existing revision for the same logical `(application_id, dserv_id)`
/// -- it only ever adds a new, additional, independently-keyed entry.
/// Registering a revision here grants it NO executable authority by
/// itself (Phase 117F's central invariant) -- it becomes active only
/// through a governed DDeploy acceptance explicitly binding to its
/// `descriptor_digest`. Returns the freshly-computed `descriptor_digest`
/// (never caller-supplied) on success, matching Phase 117E's own
/// never-trust-a-caller-digest discipline.
pub fn declare_dserv_artifact_revision(
    state: &mut DCodeArtifactRegistryStateV2,
    artifact: CanonicalDServArtifactV1,
) -> Result<String, DeclareDServArtifactRevisionError> {
    let findings = artifact.validate();
    if !findings.is_empty() {
        return Err(DeclareDServArtifactRevisionError::Invalid(findings));
    }
    let descriptor_digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(&artifact)
        .map_err(DeclareDServArtifactRevisionError::DigestComputationFailed)?;

    let key = dserv_artifact_key(&artifact.application_id, &artifact.dserv_id);
    let revisions = state.artifacts.entry(key).or_default();
    if let Some(existing) = revisions.get(&descriptor_digest) {
        if *existing == artifact {
            return Ok(descriptor_digest);
        }
        return Err(DeclareDServArtifactRevisionError::Conflict(format!(
            "descriptor_digest '{descriptor_digest}' is already registered with different content for application_id='{}' dserv_id='{}' -- this should be architecturally impossible (the digest is a content hash of the descriptor); refusing rather than silently overwriting",
            artifact.application_id, artifact.dserv_id
        )));
    }
    revisions.insert(descriptor_digest.clone(), artifact);
    Ok(descriptor_digest)
}

/// Fail-closed relational integrity check for the v2 state, mirroring
/// `validate_persisted_dcode_registry_integrity`'s v1 shape exactly, plus
/// the new invariant a versioned registry must additionally hold: every
/// inner (digest) key must equal its own stored artifact's freshly
/// recomputed descriptor digest -- never merely trusted from the
/// persisted key string.
pub fn validate_persisted_dcode_registry_integrity_v2(state: &DCodeArtifactRegistryStateV2) -> Result<(), String> {
    if state.state_version != DCODE_REGISTRY_STATE_VERSION_V2 {
        return Err(format!(
            "unsupported dcode artifact registry state_version '{}', expected '{DCODE_REGISTRY_STATE_VERSION_V2}'",
            state.state_version
        ));
    }
    for (key, revisions) in &state.artifacts {
        for (digest, artifact) in revisions {
            let expected_key = dserv_artifact_key(&artifact.application_id, &artifact.dserv_id);
            if *key != expected_key {
                return Err(format!(
                    "dcode artifact stored under logical key '{key}' does not match dserv_artifact_key('{}', '{}') = '{expected_key}'",
                    artifact.application_id, artifact.dserv_id
                ));
            }
            let recomputed = crate::core::dserv_artifact::recompute_dserv_artifact_digest(artifact)?;
            if recomputed != *digest {
                return Err(format!(
                    "dcode artifact stored under digest key '{digest}' recomputes to a different descriptor digest '{recomputed}' -- persisted state is corrupt"
                ));
            }
            let findings = artifact.validate();
            if !findings.is_empty() {
                return Err(format!(
                    "dcode artifact '{key}'/'{digest}' (application_id='{}', dserv_id='{}') no longer validates cleanly: {findings:?}",
                    artifact.application_id, artifact.dserv_id
                ));
            }
        }
    }
    Ok(())
}

/// Pure, deterministic migration from the pre-117G one-slot state. No
/// external fact is required: `descriptor_digest` is freshly recomputed
/// from each legacy artifact's own content. Every legacy entry becomes
/// exactly one immutable v2 revision -- content is preserved byte-for-byte
/// (the artifact itself is cloned unmodified; only the storage key gains a
/// digest component). Fails closed on any legacy entry that does not
/// independently validate or whose stored key does not match its own
/// recomputed logical key -- never silently repaired.
pub fn migrate_v1_state_to_v2(legacy: &DCodeArtifactRegistryStateV1) -> Result<DCodeArtifactRegistryStateV2, String> {
    validate_persisted_dcode_registry_integrity(legacy)?;
    let mut artifacts: BTreeMap<String, BTreeMap<String, CanonicalDServArtifactV1>> = BTreeMap::new();
    for (key, artifact) in &legacy.artifacts {
        let descriptor_digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(artifact)?;
        artifacts.entry(key.clone()).or_default().insert(descriptor_digest, artifact.clone());
    }
    Ok(DCodeArtifactRegistryStateV2 { state_version: DCODE_REGISTRY_STATE_VERSION_V2.to_string(), artifacts })
}

/// The ONE governed load path after Phase 117G. Transparently handles
/// both the current v2 on-disk shape and a pre-117G v1 shape (migrating it
/// in-memory, deterministically, on every load -- never silently, and
/// never persisting the migrated form itself; the next explicit
/// `persist_state_v2` call naturally upgrades the on-disk representation).
/// Fails closed on an unrecognised `state_version` or any corruption in
/// either shape.
pub fn load_state_v2(db_path: &std::path::Path) -> Result<DCodeArtifactRegistryStateV2, String> {
    match load_domain_json::<serde_json::Value>(db_path, DOMAIN_DCODE_REGISTRY)? {
        None => Ok(DCodeArtifactRegistryStateV2::empty()),
        Some(raw) => {
            let state_version = raw.get("state_version").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
            if state_version == DCODE_REGISTRY_STATE_VERSION_V2 {
                let state: DCodeArtifactRegistryStateV2 =
                    serde_json::from_value(raw).map_err(|error| format!("deserialize DCodeArtifactRegistryStateV2: {error}"))?;
                validate_persisted_dcode_registry_integrity_v2(&state)?;
                Ok(state)
            } else if state_version == DCODE_REGISTRY_STATE_VERSION {
                let legacy: DCodeArtifactRegistryStateV1 =
                    serde_json::from_value(raw).map_err(|error| format!("deserialize DCodeArtifactRegistryStateV1: {error}"))?;
                migrate_v1_state_to_v2(&legacy)
            } else {
                Err(format!("unsupported dcode artifact registry state_version '{state_version}'"))
            }
        }
    }
}

pub fn persist_state_v2(db_path: &std::path::Path, state: &DCodeArtifactRegistryStateV2) -> Result<(), String> {
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

    // ===================================================================
    // Phase 117G: versioned v2 registry test matrix (G1-G10).
    // ===================================================================

    fn temp_db_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-dcode-registry-v2-test-{tag}-{}.db", uuid::Uuid::new_v4()))
    }

    fn different_content(mut base: CanonicalDServArtifactV1) -> CanonicalDServArtifactV1 {
        base.limits.timeout_ms += 1;
        base
    }

    /// G1: migrating a legacy v1 state produces a v2 state whose sole
    /// revision for each entry is content-identical (byte-for-byte) to the
    /// legacy entry, keyed under a freshly recomputed digest, and passes
    /// the v2 integrity check.
    #[test]
    fn g1_migration_from_v1_preserves_content_and_recomputes_digest() {
        let mut legacy = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut legacy, artifact("app:a", "decode", "0.1.0")).unwrap();
        declare_dserv_artifact(&mut legacy, artifact("app:a", "alert", "0.1.0")).unwrap();

        let migrated = migrate_v1_state_to_v2(&legacy).unwrap();
        assert_eq!(migrated.state_version, DCODE_REGISTRY_STATE_VERSION_V2);
        assert!(validate_persisted_dcode_registry_integrity_v2(&migrated).is_ok());

        let expected_digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(
            legacy.get("app:a", "decode").unwrap(),
        )
        .unwrap();
        let migrated_artifact = migrated.get_exact("app:a", "decode", &expected_digest).unwrap();
        assert_eq!(migrated_artifact, legacy.get("app:a", "decode").unwrap());
        assert_eq!(migrated.revision_count("app:a", "decode"), 1);
        assert_eq!(migrated.revision_count("app:a", "alert"), 1);
    }

    /// G2: declaring the exact same content twice is idempotent -- it does
    /// not create a second revision, and returns the same digest both times.
    #[test]
    fn g2_redeclaring_identical_content_is_idempotent_and_stays_one_revision() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        let digest1 = declare_dserv_artifact_revision(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        let digest2 = declare_dserv_artifact_revision(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        assert_eq!(digest1, digest2);
        assert_eq!(state.revision_count("app:a", "svc"), 1);
    }

    /// G3: declaring genuinely different content for the SAME logical
    /// (application_id, dserv_id) adds a NEW immutable revision -- it never
    /// replaces or removes the prior one (Phase 117F's central invariant:
    /// registering D2 alone must not disturb D1).
    #[test]
    fn g3_declaring_different_content_adds_a_new_revision_without_removing_the_old_one() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        let d1 = artifact("app:a", "svc", "0.1.0");
        let digest1 = declare_dserv_artifact_revision(&mut state, d1.clone()).unwrap();
        let d2 = different_content(d1.clone());
        let digest2 = declare_dserv_artifact_revision(&mut state, d2.clone()).unwrap();

        assert_ne!(digest1, digest2);
        assert_eq!(state.revision_count("app:a", "svc"), 2);
        assert_eq!(state.get_exact("app:a", "svc", &digest1).unwrap(), &d1);
        assert_eq!(state.get_exact("app:a", "svc", &digest2).unwrap(), &d2);
    }

    /// G4: `get_exact` only ever returns the revision named by its exact
    /// digest -- an unregistered digest (even for a real logical service)
    /// returns `None`, never a fallback to some other revision.
    #[test]
    fn g4_get_exact_never_falls_back_to_a_different_revision() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        declare_dserv_artifact_revision(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        assert!(state.get_exact("app:a", "svc", &"0".repeat(64)).is_none());
        assert!(state.get_exact("app:a", "other-svc", &"0".repeat(64)).is_none());
    }

    /// G5: `get_logical_if_unambiguous` succeeds while zero or exactly one
    /// revision is registered, and fails closed (never guesses) the moment
    /// a second revision exists for the same logical service.
    #[test]
    fn g5_logical_lookup_fails_closed_once_ambiguous() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        assert!(state.get_logical_if_unambiguous("app:a", "svc").unwrap().is_none());

        let d1 = artifact("app:a", "svc", "0.1.0");
        declare_dserv_artifact_revision(&mut state, d1.clone()).unwrap();
        assert_eq!(state.get_logical_if_unambiguous("app:a", "svc").unwrap(), Some(&d1));

        declare_dserv_artifact_revision(&mut state, different_content(d1)).unwrap();
        let error = state.get_logical_if_unambiguous("app:a", "svc").unwrap_err();
        assert!(error.starts_with("dcode_artifact_lookup_ambiguous"));
    }

    /// G6: persisted-integrity validation fails closed when a stored
    /// revision's DIGEST key does not match what recomputing its own
    /// content actually produces (simulated on-disk corruption).
    #[test]
    fn g6_persisted_integrity_detects_digest_key_corruption() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        declare_dserv_artifact_revision(&mut state, artifact("app:a", "svc", "0.1.0")).unwrap();
        let key = dserv_artifact_key("app:a", "svc");
        let revisions = state.artifacts.get_mut(&key).unwrap();
        let real_digest = revisions.keys().next().unwrap().clone();
        let content = revisions.remove(&real_digest).unwrap();
        revisions.insert("0".repeat(64), content);

        let error = validate_persisted_dcode_registry_integrity_v2(&state).unwrap_err();
        assert!(error.contains("recomputes to a different descriptor digest"));
    }

    /// G7: persisted-integrity validation fails closed when a revision is
    /// stored under a logical (outer) key that does not match its own
    /// content's `dserv_artifact_key`.
    #[test]
    fn g7_persisted_integrity_detects_logical_key_corruption() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        let a = artifact("app:a", "svc", "0.1.0");
        let digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(&a).unwrap();
        state.artifacts.entry("wrong-logical-key".to_string()).or_default().insert(digest, a);

        let error = validate_persisted_dcode_registry_integrity_v2(&state).unwrap_err();
        assert!(error.contains("does not match dserv_artifact_key"));
    }

    /// G8: persisted-integrity validation fails closed when a stored
    /// revision no longer independently validates.
    #[test]
    fn g8_persisted_integrity_detects_invalid_stored_artifact() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        let mut bad = artifact("app:a", "svc", "0.1.0");
        bad.abi.allowed_imports.push("evil".to_string());
        let key = dserv_artifact_key("app:a", "svc");
        let digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(&bad).unwrap();
        // Bypass `declare_dserv_artifact_revision`'s own validation gate to
        // simulate corruption that occurred after a valid declaration --
        // stored under its OWN correctly-recomputed digest, so the digest
        // check passes and the content-validity check is what fires.
        state.artifacts.entry(key).or_default().insert(digest, bad);

        let error = validate_persisted_dcode_registry_integrity_v2(&state).unwrap_err();
        assert!(error.contains("no longer validates cleanly"));
    }

    /// G9: an unrecognised `state_version` fails closed on v2 reload rather
    /// than being silently treated as either shape.
    #[test]
    fn g9_unsupported_state_version_fails_closed_on_v2_reload() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        state.state_version = "bogus".to_string();
        let error = validate_persisted_dcode_registry_integrity_v2(&state).unwrap_err();
        assert!(error.contains("unsupported dcode artifact registry state_version"));
    }

    /// G10: `load_state_v2`/`persist_state_v2` round-trip a multi-revision
    /// registry unchanged, and `load_state_v2` transparently upgrades a
    /// pre-117G v1 on-disk state in memory (without requiring a separate
    /// migration step to be called by the reader).
    #[test]
    fn g10_load_state_v2_round_trips_and_transparently_migrates_legacy_v1_on_disk() {
        let db_path = temp_db_path("roundtrip");

        // Fresh v2 round-trip with two coexisting revisions.
        let mut state = DCodeArtifactRegistryStateV2::empty();
        let d1 = artifact("app:a", "svc", "0.1.0");
        declare_dserv_artifact_revision(&mut state, d1.clone()).unwrap();
        declare_dserv_artifact_revision(&mut state, different_content(d1)).unwrap();
        persist_state_v2(&db_path, &state).unwrap();
        let reloaded = load_state_v2(&db_path).unwrap();
        assert_eq!(reloaded, state);
        assert_eq!(reloaded.revision_count("app:a", "svc"), 2);

        // A pre-117G v1 file on disk loads transparently as v2.
        let db_path_legacy = temp_db_path("legacy");
        let mut legacy = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut legacy, artifact("app:a", "decode", "0.1.0")).unwrap();
        persist_state(&db_path_legacy, &legacy).unwrap();
        let migrated = load_state_v2(&db_path_legacy).unwrap();
        assert_eq!(migrated.state_version, DCODE_REGISTRY_STATE_VERSION_V2);
        assert_eq!(migrated.revision_count("app:a", "decode"), 1);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&db_path_legacy);
    }

    /// Section 0B / project-reuse: the registry is deliberately project-less
    /// (no `project_id` in its key). Two different projects deploying the
    /// EXACT same `(application_id, dserv_id)` content share ONE registry
    /// entry (safe, ordinary content-addressed deduplication) -- there is no
    /// project-scoped duplication anywhere in this module. Project scoping
    /// belongs entirely to DDeploy's own proposal/acceptance binding layer,
    /// which is exercised separately in `ddeploy.rs`'s own project-scoped
    /// (`project_id`-bearing) proposal tests.
    #[test]
    fn project_reuse_same_content_shares_one_registry_entry() {
        let mut state = DCodeArtifactRegistryStateV2::empty();
        // Two DIFFERENT projects both declare byte-identical content for the
        // same (application_id, dserv_id) -- the registry has no notion of
        // "project" at all, so this is simply the same declaration twice.
        let digest_from_project_a = declare_dserv_artifact_revision(&mut state, artifact("shared-app", "shared-svc", "0.1.0")).unwrap();
        let digest_from_project_b = declare_dserv_artifact_revision(&mut state, artifact("shared-app", "shared-svc", "0.1.0")).unwrap();
        assert_eq!(digest_from_project_a, digest_from_project_b);
        assert_eq!(state.revision_count("shared-app", "shared-svc"), 1, "identical content declared under the same logical service must never duplicate, regardless of which project's DDeploy proposal will later bind to it");
    }
}
