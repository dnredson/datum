//! `datum.dserv-artifact/1` (Stage 111-ext / ADR-0021) -- the canonical,
//! host-independent, content-addressed identity of a D-Serv's D-Code
//! (ADR-0005: "Application WASM / D-Code", never the legacy Management WASM
//! path). This is the FIRST Rust type for this schema string: prior to this
//! module it existed only as hand-authored Stage 107 fixtures
//! (`scripts/fixtures/datum-contracts/vs1/dserv-artifact.*.json`) and ADR
//! prose (ADR-0003/ADR-0004/ADR-0005/ADR-0008). Every field below mirrors
//! those fixtures exactly -- no required field was renamed, removed, or
//! reinterpreted; this is a formalization, not a redesign.
//!
//! `#[serde(deny_unknown_fields)]` on the outer type is deliberate and
//! load-bearing: ADR-0004 invariant 10 forbids ANY host-kind field on this
//! contract (`negative/dserv-artifact.host-kind-field.json` adds
//! `host_kind`/`runtime` and must be rejected). Denying unknown fields at
//! the wire boundary is the cheapest, most direct way to enforce that
//! invariant for every present and future illegal field, not just the two
//! named in that one fixture.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::canonical_dgraph::{CanonicalValidationFinding, DNODE_ABI_V0};
use crate::core::jcs;

pub const CANONICAL_DSERV_ARTIFACT_SCHEMA: &str = "datum.dserv-artifact/1";

/// ADR-0003's `datum-dnode/0` required export surface -- the ONLY concrete
/// calling convention this repository's ADRs actually define (ADR-0003
/// section "Decision"). v0.1 D-Code implements this exactly; it is NOT the
/// generic scalar-argument ABI used by the closed Controlled WASM
/// Deployment ("Management WASM") phase, and it is NOT an invented
/// alternative -- see `documentation/adr/ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md`
/// for the explicit "ADR-0003 already defines this; do not invent a
/// different one" audit trail.
pub const DNODE_ABI_V0_REQUIRED_EXPORTS: &[&str] =
    &["memory", "datum_abi_version", "datum_alloc", "datum_dealloc", "datum_handle"];

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDServArtifactV1 {
    pub schema: String,
    pub artifact_id: String,
    pub version: String,
    pub application_id: String,
    pub dserv_id: String,
    pub dcode: DServArtifactDCodeV1,
    pub abi: DServArtifactAbiV1,
    pub ports: DServArtifactPortsV1,
    pub limits: DServArtifactLimitsV1,
    pub safety: DServArtifactSafetyV1,
    pub state_model: String,
    pub instantiation: String,
    #[serde(default)]
    pub constraints: DServArtifactConstraintsV1,
    #[serde(default)]
    pub qos: Value,
    #[serde(default)]
    pub provenance: Value,
    #[serde(default)]
    pub probes: Value,
    #[serde(default)]
    pub signature_ref: Option<String>,
}

/// Content-addressed D-Code module identity. `sha256` and `content_id` are
/// deliberately two fields (matching the checked-in fixtures exactly) but
/// must always agree -- `validate()` rejects a mismatch. The bytes actually
/// executed are verified against this digest at load time
/// (`dcode_registry`/DATUM `dcode::identity`), never trusted from a
/// filename, path, or caller claim.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DServArtifactDCodeV1 {
    pub uri: String,
    pub sha256: String,
    pub content_id: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DServArtifactAbiV1 {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub required_exports: Vec<String>,
    /// v0.1 policy (ADR-0003 Decision, this ADR-0021 slice): must be empty.
    /// A module importing anything -- WASI, clocks, sockets, filesystem --
    /// fails validation before it is ever compiled/instantiated.
    #[serde(default)]
    pub allowed_imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DServArtifactPortsV1 {
    #[serde(default)]
    pub inputs: Vec<DServArtifactPortV1>,
    #[serde(default)]
    pub outputs: Vec<DServArtifactPortV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DServArtifactPortV1 {
    pub port: String,
    pub payload_schema: String,
    #[serde(default)]
    pub external: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DServArtifactLimitsV1 {
    pub fuel_per_invocation: u64,
    pub timeout_ms: u64,
    pub max_memory_pages: u32,
    pub max_message_bytes: u32,
    pub max_emits_per_invocation: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DServArtifactSafetyV1 {
    #[serde(default)]
    pub filesystem_access: bool,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub host_command_access: bool,
    #[serde(default)]
    pub docker_access: bool,
    #[serde(default)]
    pub side_effects_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DServArtifactConstraintsV1 {
    #[serde(default)]
    pub allowed_stages: Vec<String>,
}

impl CanonicalDServArtifactV1 {
    /// Structural, self-contained validation -- never touches the D-Graph,
    /// D-Node registry, or any other contract. Collects every finding;
    /// returns `Vec::new()` iff valid.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DSERV_ARTIFACT_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!("schema must be '{CANONICAL_DSERV_ARTIFACT_SCHEMA}', got '{}'", self.schema),
            ));
        }
        for (field, value) in [
            ("artifact_id", &self.artifact_id),
            ("version", &self.version),
            ("application_id", &self.application_id),
            ("dserv_id", &self.dserv_id),
        ] {
            if value.trim().is_empty() {
                findings.push(finding("missing_field", format!("{field} must be a non-empty string")));
            }
        }

        match (parse_sha256(&self.dcode.sha256), parse_sha256_content_id(&self.dcode.content_id)) {
            (Some(sha), Some(content)) if sha == content => {}
            (Some(_), Some(_)) => findings.push(finding(
                "dcode_identity_mismatch",
                "dcode.sha256 and dcode.content_id must reference the identical digest",
            )),
            _ => findings.push(finding(
                "dcode_identity_malformed",
                "dcode.sha256 and dcode.content_id must both be 'sha256:<64 lowercase hex>' (content_id) / 64 lowercase hex (sha256), matching each other",
            )),
        }

        // Independent review v3 (F7A): a descriptor must not name THREE
        // different module content identities. Every real, checked-in
        // fixture already uses `datum-blob://sha256/<64hex>` with that
        // digest equal to both `dcode.sha256` and `dcode.content_id`; no
        // accepted ADR documents an alternate URI scheme, so this tightens
        // rather than contradicts existing accepted behavior.
        match parse_datum_blob_uri_digest(&self.dcode.uri) {
            Some(uri_digest) => {
                let sha = parse_sha256(&self.dcode.sha256);
                let content = parse_sha256_content_id(&self.dcode.content_id);
                if sha.as_deref() != Some(uri_digest.as_str()) || content.as_deref() != Some(uri_digest.as_str()) {
                    findings.push(finding(
                        "dcode_uri_identity_mismatch",
                        "dcode.uri digest must equal both dcode.sha256 and dcode.content_id -- a descriptor must not name three different module content identities",
                    ));
                }
            }
            None => findings.push(finding(
                "dcode_uri_malformed",
                "dcode.uri must be 'datum-blob://sha256/<64 lowercase hex>'",
            )),
        }

        let expected_abi = format!("{}/{}", self.abi.name, self.abi.version);
        if expected_abi != DNODE_ABI_V0 {
            findings.push(finding(
                "dnode_abi_unsupported",
                format!("abi '{expected_abi}' is not a supported D-Node ABI; v0.1 supports only '{DNODE_ABI_V0}'"),
            ));
        }
        for required in DNODE_ABI_V0_REQUIRED_EXPORTS {
            if !self.abi.required_exports.iter().any(|export| export == required) {
                findings.push(finding(
                    "dnode_abi_export_missing",
                    format!("abi.required_exports is missing required export '{required}' for '{DNODE_ABI_V0}'"),
                ));
            }
        }
        // Independent review v3 (F7B): require the EXACT semantic set, not
        // merely a superset -- reject duplicates and unknown extras. Every
        // real, checked-in fixture already lists exactly the 5 required
        // exports with no duplicates/extras; no accepted ADR documents a
        // superset or alternate calling convention, so this tightens rather
        // than contradicts existing accepted behavior.
        {
            let mut seen = std::collections::BTreeSet::new();
            let mut has_duplicate = false;
            for export in &self.abi.required_exports {
                if !seen.insert(export.as_str()) {
                    has_duplicate = true;
                }
            }
            if has_duplicate {
                findings.push(finding(
                    "dnode_abi_export_duplicate",
                    "abi.required_exports must not contain duplicate entries",
                ));
            }
            let canonical_set: std::collections::BTreeSet<&str> = DNODE_ABI_V0_REQUIRED_EXPORTS.iter().copied().collect();
            let unknown: Vec<&str> = seen.iter().filter(|export| !canonical_set.contains(*export)).copied().collect();
            if !unknown.is_empty() {
                findings.push(finding(
                    "dnode_abi_export_unknown",
                    format!(
                        "abi.required_exports contains export(s) not part of the datum-dnode/0 required set: {unknown:?}"
                    ),
                ));
            }
        }
        if !self.abi.allowed_imports.is_empty() {
            findings.push(finding(
                "dnode_abi_imports_forbidden",
                "abi.allowed_imports must be empty in v0.1 -- a D-Code module may declare zero host imports",
            ));
        }

        if self.limits.fuel_per_invocation == 0 {
            findings.push(finding("limits_invalid", "limits.fuel_per_invocation must be greater than zero"));
        }
        if self.limits.timeout_ms == 0 {
            findings.push(finding("limits_invalid", "limits.timeout_ms must be greater than zero"));
        }
        if self.limits.max_memory_pages == 0 {
            findings.push(finding("limits_invalid", "limits.max_memory_pages must be greater than zero"));
        }
        if self.limits.max_message_bytes == 0 {
            findings.push(finding("limits_invalid", "limits.max_message_bytes must be greater than zero"));
        }
        if self.limits.max_emits_per_invocation == 0 {
            findings.push(finding("limits_invalid", "limits.max_emits_per_invocation must be greater than zero"));
        }

        // Independent review v2 (R7): every `safety` field is declarative
        // metadata about a capability a v0.1 zero-import D-Code module could
        // exercise -- but v0.1 forbids ALL host imports (`abi.allowed_imports`
        // must be empty, enforced above), so a module has no possible
        // mechanism to touch a filesystem, network, host command, Docker, or
        // any other side effect regardless of what this field claims.
        // Declaring any of them `true` is therefore either a lie about the
        // artifact's own real capabilities or evidence the artifact was
        // authored against a different (non-zero-import) execution model;
        // both are rejected structurally rather than silently accepted and
        // then never actually enforced at the WASM boundary.
        for (field_name, value) in [
            ("filesystem_access", self.safety.filesystem_access),
            ("network_access", self.safety.network_access),
            ("host_command_access", self.safety.host_command_access),
            ("docker_access", self.safety.docker_access),
            ("side_effects_allowed", self.safety.side_effects_allowed),
        ] {
            if value {
                findings.push(finding(
                    "safety_capability_impossible_for_zero_import_dcode",
                    format!(
                        "safety.{field_name} must be false -- a zero-import v0.1 D-Code module has no possible mechanism to exercise this capability"
                    ),
                ));
            }
        }

        if self.state_model != "stateless" {
            findings.push(finding(
                "state_model_unsupported",
                format!("state_model '{}' is not supported; v0.1 supports only 'stateless'", self.state_model),
            ));
        }
        if self.instantiation != "fresh_instance_per_invocation" {
            findings.push(finding(
                "instantiation_unsupported",
                format!(
                    "instantiation '{}' is not supported; ADR-0008 requires 'fresh_instance_per_invocation'",
                    self.instantiation
                ),
            ));
        }

        findings
    }
}

fn parse_sha256(value: &str) -> Option<String> {
    let digest = value.strip_prefix("sha256:").unwrap_or(value);
    if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        Some(digest.to_string())
    } else {
        None
    }
}

fn parse_sha256_content_id(value: &str) -> Option<String> {
    let digest = value.strip_prefix("sha256:")?;
    if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        Some(digest.to_string())
    } else {
        None
    }
}

/// F7A: parses `datum-blob://sha256/<64 lowercase hex>`, returning the
/// digest component. This is the ONLY URI form any accepted fixture/ADR
/// documents for `dcode.uri`.
fn parse_datum_blob_uri_digest(uri: &str) -> Option<String> {
    let digest = uri.strip_prefix("datum-blob://sha256/")?;
    if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        Some(digest.to_string())
    } else {
        None
    }
}

/// Canonical JCS+SHA-256 digest (ADR-0006). `required_exports`/
/// `allowed_imports` are canonically UNORDERED SETS (mirroring the
/// `dnode_abi_versions` convention in `dnode_registry.rs`): a deterministic
/// sorted copy is hashed so two semantically identical artifacts that list
/// exports/imports in a different order produce the identical digest.
pub fn recompute_dserv_artifact_digest(artifact: &CanonicalDServArtifactV1) -> Result<String, String> {
    let mut sorted = artifact.clone();
    sorted.abi.required_exports.sort();
    sorted.abi.allowed_imports.sort();
    let value: Value = serde_json::to_value(&sorted)
        .map_err(|error| format!("serialize CanonicalDServArtifactV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CanonicalDServArtifactV1 {
        CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: "dserv:sensor-pipeline:decode".to_string(),
            version: "0.1.0".to_string(),
            application_id: "app:sensor-pipeline".to_string(),
            dserv_id: "decode".to_string(),
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{}", "a".repeat(64)),
                sha256: format!("{}", "a".repeat(64)),
                content_id: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 128,
            },
            abi: DServArtifactAbiV1 {
                name: "datum-dnode".to_string(),
                version: "0".to_string(),
                required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
                allowed_imports: Vec::new(),
            },
            ports: DServArtifactPortsV1::default(),
            limits: DServArtifactLimitsV1 {
                fuel_per_invocation: 5_000_000,
                timeout_ms: 200,
                max_memory_pages: 64,
                max_message_bytes: 65536,
                max_emits_per_invocation: 8,
            },
            safety: DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: DServArtifactConstraintsV1::default(),
            qos: Value::Object(Default::default()),
            provenance: Value::Null,
            probes: Value::Null,
            signature_ref: None,
        }
    }

    #[test]
    fn valid_artifact_validates_clean() {
        assert!(sample().validate().is_empty());
    }

    #[test]
    fn real_fixture_decode_validates_clean() {
        let json = std::fs::read_to_string(
            "../scripts/fixtures/datum-contracts/vs1/dserv-artifact.decode.json",
        )
        .expect("fixture should be readable from dserver/");
        let artifact: CanonicalDServArtifactV1 =
            serde_json::from_str(&json).expect("fixture should deserialize into the typed contract");
        assert!(artifact.validate().is_empty(), "fixture should validate cleanly: {:?}", artifact.validate());
    }

    #[test]
    fn real_fixture_alert_validates_clean() {
        let json = std::fs::read_to_string(
            "../scripts/fixtures/datum-contracts/vs1/dserv-artifact.alert.json",
        )
        .expect("fixture should be readable from dserver/");
        let artifact: CanonicalDServArtifactV1 =
            serde_json::from_str(&json).expect("fixture should deserialize into the typed contract");
        assert!(artifact.validate().is_empty(), "fixture should validate cleanly: {:?}", artifact.validate());
    }

    #[test]
    fn host_kind_field_fixture_is_rejected_at_deserialization() {
        let json = std::fs::read_to_string(
            "../scripts/fixtures/datum-contracts/vs1/negative/dserv-artifact.host-kind-field.json",
        )
        .expect("fixture should be readable from dserver/");
        let result: Result<CanonicalDServArtifactV1, _> = serde_json::from_str(&json);
        assert!(result.is_err(), "an artifact with an illegal host_kind/runtime field must be rejected");
    }

    #[test]
    fn wrong_schema_is_rejected() {
        let mut artifact = sample();
        artifact.schema = "datum.dserv-artifact/2".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn mismatched_sha256_and_content_id_is_rejected() {
        let mut artifact = sample();
        artifact.dcode.content_id = format!("sha256:{}", "b".repeat(64));
        assert!(artifact.validate().iter().any(|f| f.code == "dcode_identity_mismatch"));
    }

    #[test]
    fn malformed_digest_is_rejected() {
        let mut artifact = sample();
        artifact.dcode.sha256 = "not-a-digest".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "dcode_identity_malformed"));
    }

    #[test]
    fn unsupported_abi_is_rejected() {
        let mut artifact = sample();
        artifact.abi.version = "1".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_unsupported"));
    }

    #[test]
    fn missing_required_export_is_rejected() {
        let mut artifact = sample();
        artifact.abi.required_exports.retain(|export| export != "datum_handle");
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_export_missing"));
    }

    /// F7A: uri digest disagreeing with sha256/content_id must be rejected
    /// -- a descriptor must not name three different module identities.
    #[test]
    fn dcode_uri_digest_mismatch_is_rejected() {
        let mut artifact = sample();
        artifact.dcode.uri = format!("datum-blob://sha256/{}", "b".repeat(64));
        assert!(artifact.validate().iter().any(|f| f.code == "dcode_uri_identity_mismatch"));
    }

    /// F7A: a URI not in the one documented `datum-blob://sha256/<64hex>`
    /// form is rejected.
    #[test]
    fn dcode_uri_malformed_is_rejected() {
        let mut artifact = sample();
        artifact.dcode.uri = "https://example.com/module.wasm".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "dcode_uri_malformed"));
    }

    /// F7B: a duplicate required export is rejected even though every
    /// individually-required export is still present.
    #[test]
    fn duplicate_required_export_is_rejected() {
        let mut artifact = sample();
        artifact.abi.required_exports.push("memory".to_string());
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_export_duplicate"));
    }

    /// F7B: an unknown extra export -- not part of the datum-dnode/0
    /// required set -- is rejected, not silently tolerated as a superset.
    #[test]
    fn unknown_required_export_is_rejected() {
        let mut artifact = sample();
        artifact.abi.required_exports.push("datum_extra_unofficial_export".to_string());
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_export_unknown"));
    }

    #[test]
    fn nonempty_allowed_imports_is_rejected() {
        let mut artifact = sample();
        artifact.abi.allowed_imports.push("wasi_snapshot_preview1".to_string());
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_imports_forbidden"));
    }

    #[test]
    fn zero_fuel_limit_is_rejected() {
        let mut artifact = sample();
        artifact.limits.fuel_per_invocation = 0;
        assert!(artifact.validate().iter().any(|f| f.code == "limits_invalid"));
    }

    /// Independent review v2, R7.
    #[test]
    fn zero_max_emits_limit_is_rejected() {
        let mut artifact = sample();
        artifact.limits.max_emits_per_invocation = 0;
        assert!(artifact.validate().iter().any(|f| f.code == "limits_invalid"));
    }

    /// Independent review v2, R7: every individual safety flag, declared
    /// true, must be rejected -- a zero-import v0.1 D-Code module has no
    /// mechanism to exercise any of them regardless of what it claims.
    #[test]
    fn each_safety_flag_declared_true_is_rejected() {
        for flip in [
            |a: &mut CanonicalDServArtifactV1| a.safety.filesystem_access = true,
            |a: &mut CanonicalDServArtifactV1| a.safety.network_access = true,
            |a: &mut CanonicalDServArtifactV1| a.safety.host_command_access = true,
            |a: &mut CanonicalDServArtifactV1| a.safety.docker_access = true,
            |a: &mut CanonicalDServArtifactV1| a.safety.side_effects_allowed = true,
        ] {
            let mut artifact = sample();
            flip(&mut artifact);
            assert!(
                artifact.validate().iter().any(|f| f.code == "safety_capability_impossible_for_zero_import_dcode"),
                "expected rejection for {artifact:?}"
            );
        }
    }

    #[test]
    fn all_safety_flags_false_is_accepted() {
        assert!(sample().validate().is_empty());
    }

    #[test]
    fn stateful_state_model_is_rejected() {
        let mut artifact = sample();
        artifact.state_model = "stateful".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "state_model_unsupported"));
    }

    #[test]
    fn non_fresh_instantiation_is_rejected() {
        let mut artifact = sample();
        artifact.instantiation = "warm_pool".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "instantiation_unsupported"));
    }

    #[test]
    fn digest_is_deterministic_regardless_of_export_import_order() {
        let mut a = sample();
        let mut b = sample();
        a.abi.required_exports.reverse();
        b.abi.required_exports.rotate_left(2);
        assert_eq!(
            recompute_dserv_artifact_digest(&a).unwrap(),
            recompute_dserv_artifact_digest(&b).unwrap()
        );
    }

    #[test]
    fn digest_changes_when_content_changes() {
        let a = sample();
        let mut b = sample();
        b.version = "0.2.0".to_string();
        assert_ne!(
            recompute_dserv_artifact_digest(&a).unwrap(),
            recompute_dserv_artifact_digest(&b).unwrap()
        );
    }
}
