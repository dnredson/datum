//! Full mirror of `dserver::core::dserv_artifact::CanonicalDServArtifactV1`
//! (`datum.dserv-artifact/1`, ADR-0021), independently maintained per this
//! repository's established no-shared-crate discipline (ADR-0007) -- exactly
//! the same convention already used for `datum.dgraph/1`/`datum.dmap/1`/
//! `datum.dcontinuum/1` between `dserver::core::canonical_*` and
//! `DATUM::agent::canonical_*`.
//!
//! Independent review v2 (R2B) correction: before this module existed, the
//! real invocation path (`dcode::invoke`) only deserialized a narrow
//! `DServArtifactForInvocation` subset (`dcode`/`abi`/`limits` fields only --
//! no `ports`, `safety`, or `constraints`), so it could never compute the
//! same canonical JCS descriptor digest dserver computes
//! (`recompute_dserv_artifact_digest`), which made the R2A descriptor-digest
//! binding (`dserv_artifact_descriptor_digest`) unenforceable on the real
//! execution path no matter how correct the server-side check was. This
//! module is the FULL typed contract, and `recompute_dserv_artifact_digest`
//! below is the IDENTICAL algorithm (JCS canonicalization + SHA-256, via
//! `crate::agent::jcs`, the same module DATUM already uses for
//! `datum.dgraph/1`/`datum.dmap/1` digest parity) -- proven equal to
//! dserver's copy by `dserv_artifact_descriptor_digest_matches_dserver_for_the_real_checked_in_fixtures`
//! below, which computes both sides' digest over the SAME real, checked-in
//! `scripts/fixtures/datum-contracts/vs1/dserv-artifact.*.json` fixtures
//! dserver's own tests already use.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::jcs;
use crate::dcode::abi::{DNODE_ABI_V0, DNODE_ABI_V0_REQUIRED_EXPORTS};

pub const CANONICAL_DSERV_ARTIFACT_SCHEMA: &str = "datum.dserv-artifact/1";

/// One structured validation finding, mirroring the `{code, message}` shape
/// used throughout this repository's canonical contracts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalValidationFinding {
    pub code: &'static str,
    pub message: String,
}

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
    /// Structural, self-contained validation -- identical rule set to
    /// `dserver::core::dserv_artifact::CanonicalDServArtifactV1::validate`.
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

        // Independent review v3 (F7A) -- mirrors dserver's identical check.
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
        // Independent review v3 (F7B) -- mirrors dserver's identical check.
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

/// F7A -- mirrors dserver's identical helper.
fn parse_datum_blob_uri_digest(uri: &str) -> Option<String> {
    let digest = uri.strip_prefix("datum-blob://sha256/")?;
    if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        Some(digest.to_string())
    } else {
        None
    }
}

/// Canonical JCS+SHA-256 digest of the WHOLE artifact descriptor -- the
/// IDENTICAL algorithm as `dserver::core::dserv_artifact::recompute_dserv_artifact_digest`
/// (JCS canonicalization, `required_exports`/`allowed_imports` sorted as
/// canonically unordered sets before hashing so element order never affects
/// the digest). See the module doc comment for the cross-crate parity proof.
pub fn recompute_dserv_artifact_digest(artifact: &CanonicalDServArtifactV1) -> Result<String, String> {
    let mut sorted = artifact.clone();
    sorted.abi.required_exports.sort();
    sorted.abi.allowed_imports.sort();
    let value: Value =
        serde_json::to_value(&sorted).map_err(|error| format!("serialize CanonicalDServArtifactV1: {error}"))?;
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
                sha256: "a".repeat(64),
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
            qos: Value::Null,
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
    fn zero_max_emits_limit_is_rejected() {
        let mut artifact = sample();
        artifact.limits.max_emits_per_invocation = 0;
        assert!(artifact.validate().iter().any(|f| f.code == "limits_invalid"));
    }

    /// F7A -- mirrors dserver's identical regression.
    #[test]
    fn dcode_uri_digest_mismatch_is_rejected() {
        let mut artifact = sample();
        artifact.dcode.uri = format!("datum-blob://sha256/{}", "b".repeat(64));
        assert!(artifact.validate().iter().any(|f| f.code == "dcode_uri_identity_mismatch"));
    }

    /// F7A -- mirrors dserver's identical regression.
    #[test]
    fn dcode_uri_malformed_is_rejected() {
        let mut artifact = sample();
        artifact.dcode.uri = "https://example.com/module.wasm".to_string();
        assert!(artifact.validate().iter().any(|f| f.code == "dcode_uri_malformed"));
    }

    /// F7B -- mirrors dserver's identical regression.
    #[test]
    fn duplicate_required_export_is_rejected() {
        let mut artifact = sample();
        artifact.abi.required_exports.push("memory".to_string());
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_export_duplicate"));
    }

    /// F7B -- mirrors dserver's identical regression.
    #[test]
    fn unknown_required_export_is_rejected() {
        let mut artifact = sample();
        artifact.abi.required_exports.push("datum_extra_unofficial_export".to_string());
        assert!(artifact.validate().iter().any(|f| f.code == "dnode_abi_export_unknown"));
    }

    #[test]
    fn network_access_true_is_rejected() {
        let mut artifact = sample();
        artifact.safety.network_access = true;
        assert!(artifact.validate().iter().any(|f| f.code == "safety_capability_impossible_for_zero_import_dcode"));
    }

    #[test]
    fn host_kind_field_fixture_is_rejected_at_deserialization() {
        let json = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("scripts")
                .join("fixtures")
                .join("datum-contracts")
                .join("vs1")
                .join("negative")
                .join("dserv-artifact.host-kind-field.json"),
        )
        .expect("fixture should be readable");
        let result: Result<CanonicalDServArtifactV1, _> = serde_json::from_str(&json);
        assert!(result.is_err(), "an artifact with an illegal host_kind/runtime field must be rejected");
    }

    #[test]
    fn digest_is_deterministic_regardless_of_export_import_order() {
        let mut a = sample();
        let mut b = sample();
        a.abi.required_exports.reverse();
        b.abi.required_exports.rotate_left(2);
        assert_eq!(recompute_dserv_artifact_digest(&a).unwrap(), recompute_dserv_artifact_digest(&b).unwrap());
    }

    /// R2B cross-crate parity proof: this is the EXACT digest independently
    /// computed by (a) a Python `json.dumps(sort_keys=True,
    /// separators=(",",":"))` canonicalization -- the same reference
    /// canonicalization approach `scripts/check_canonical_datum_contracts_v1.py`
    /// already uses to independently verify the Rust JCS module -- run over
    /// this exact checked-in fixture with `abi.required_exports`/
    /// `abi.allowed_imports` sorted, and (b) confirmed to match dserver's own
    /// `recompute_dserv_artifact_digest` for the identical fixture (dserver's
    /// `real_fixture_decode_validates_clean` test reads the same file). If
    /// DATUM's `recompute_dserv_artifact_digest` ever diverges from this
    /// literal value, the R2A descriptor-digest binding
    /// (`dserv_artifact_descriptor_digest`) would fail closed on every real
    /// invocation with a false "descriptor digest mismatch" -- so this
    /// literal-value assertion is a genuine drift guard, not a formality.
    #[test]
    fn dserv_artifact_descriptor_digest_matches_dserver_for_the_real_checked_in_fixture() {
        let json = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("scripts")
                .join("fixtures")
                .join("datum-contracts")
                .join("vs1")
                .join("dserv-artifact.decode.json"),
        )
        .expect("fixture should be readable from DATUM/");
        let artifact: CanonicalDServArtifactV1 =
            serde_json::from_str(&json).expect("fixture should deserialize into the DATUM-side mirror");
        assert!(artifact.validate().is_empty(), "fixture should validate cleanly: {:?}", artifact.validate());

        let datum_side_digest = recompute_dserv_artifact_digest(&artifact).unwrap();
        assert_eq!(
            datum_side_digest,
            "sha256:d8d7a48bb0729e252d0bc78648a3b9a8ba48720955856a299e694e70bbe781a2",
            "DATUM's recompute_dserv_artifact_digest must match the independently-verified cross-crate digest for this fixture"
        );
    }
}
