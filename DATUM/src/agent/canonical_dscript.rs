//! `datum.dscript/1` (Phase 116B) -- the canonical COMPILE-TIME source
//! representation of a D-Application (ADR-0001): a D-Script is "a set of
//! D-Functions with non-functional annotations." D-Compile (Phase 117, not
//! yet implemented) will map this into a D-Graph and D-Code, with
//! D-Function -> D-Serv cardinality that may be 1:1, 1:N, M:1, or M:N.
//!
//! Independently mirrored (not shared) by `dserver::core::canonical_dscript`,
//! per this project's established no-shared-crate discipline (ADR-0007):
//! parity is proven by a real cross-crate fixture + test on each side, never
//! by type sharing. See that module's doc comment for the full rationale --
//! why this is a new, deliberately minimal v1 contract, why the obsolete VS1
//! fixture sketch is not its basis, and why non-functional annotations are
//! represented as an opaque `BTreeMap<String, String>` rather than an
//! unconstrained `serde_json::Value` catch-all or any invented domain
//! semantics (CPU/memory/QoS/placement/retries/timeouts/readiness).
//!
//! What this type deliberately does NOT do (Phase 116A negative-space
//! findings): no parser/compiler/runtime, no D-Graph/D-Call generation, no
//! placement field, no executable identity, no D-Graph topology of its own,
//! no observed-runtime-state field, no control-flow or failure/recovery
//! construct, and no `dscript_digest` field on the type itself
//! (`recompute_dscript_digest` below is a free function, mirroring
//! `canonical_dgraph::recompute_dgraph_digest`'s own precedent).
//!
//! **Phase 116C: `#[serde(deny_unknown_fields)]`.** Added to both types here
//! too, for WIRE-CONTRACT PARITY with `dserver::core::canonical_dscript`
//! (see that module's doc comment for the full rationale: the newer,
//! stricter Phase-112+ canonical convention, ADR-0024). This crate
//! deliberately does NOT add `dserver::core::canonical_dscript`'s
//! `decode_and_validate_dscript_json`/`DScriptInputError`/
//! `ValidatedDScriptInput` pure input boundary: per
//! `documentation/reference/phase116c-dscript-input-admission-and-closure.md`,
//! Phase 117 DCompile is expected to be server-side, and this crate has no
//! proven consumer for that convenience boundary. Contract parity (the wire
//! shape, including `deny_unknown_fields`) is mandatory and proven below;
//! parity of every helper function on top of it is not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::jcs;

pub const CANONICAL_DSCRIPT_SCHEMA: &str = "datum.dscript/1";

/// One structured validation finding, mirroring the `{code, message}` shape
/// `canonical_dgraph::CanonicalValidationFinding` already uses. Defined
/// locally, not imported, because a D-Script's v1 validation is fully
/// self-contained and never touches another canonical contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalValidationFinding {
    pub code: &'static str,
    pub message: String,
}

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDScriptV1 {
    pub schema: String,
    /// ADR-0009 precedent (`canonical_dgraph::CanonicalDGraphV1::application_id`):
    /// distinct from `project_id`. A D-Script never carries a `project_id`
    /// itself.
    pub application_id: String,
    pub dscript_id: String,
    pub revision: u64,
    /// A SET of D-Functions (ADR-0001's own wording), serialized as a `Vec`
    /// purely as a wire-format container. Empty is structurally legal.
    #[serde(default)]
    pub functions: Vec<CanonicalDFunctionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDFunctionV1 {
    /// Stable identity, matching this project's `<noun>_id` convention
    /// rather than a display name.
    pub function_id: String,
    /// Deliberately conservative, opaque name->value string map. Never
    /// interpreted by this module. See the module doc comment.
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
}

impl CanonicalDScriptV1 {
    /// Structural, self-contained validation -- never touches another
    /// contract. Collects every finding; returns `Vec::new()` iff valid.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DSCRIPT_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!(
                    "schema must be '{CANONICAL_DSCRIPT_SCHEMA}', got '{}'",
                    self.schema
                ),
            ));
        }
        if self.application_id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "application_id must be a non-empty string",
            ));
        }
        if self.dscript_id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "dscript_id must be a non-empty string",
            ));
        }
        if self.revision < 1 {
            findings.push(finding(
                "revision_invalid",
                "revision must be an integer >= 1",
            ));
        }

        let mut seen_function_ids = std::collections::HashSet::new();
        for function in &self.functions {
            if function.function_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "function_id must be a non-empty string",
                ));
            } else if !seen_function_ids.insert(function.function_id.as_str()) {
                findings.push(finding(
                    "duplicate_function_id",
                    format!("duplicate function_id '{}'", function.function_id),
                ));
            }
            for key in function.annotations.keys() {
                if key.trim().is_empty() {
                    findings.push(finding(
                        "missing_field",
                        format!(
                            "function '{}' has an annotation with an empty key",
                            function.function_id
                        ),
                    ));
                }
            }
        }

        findings
    }
}

/// Canonical JCS+SHA-256 digest (ADR-0006), never self-referential.
/// `functions` is a canonically UNORDERED SET keyed by `function_id`,
/// mirroring `canonical_dgraph`'s own ordering decision: a deterministic
/// sorted copy is hashed, so reordering the array never changes the digest.
pub fn recompute_dscript_digest(dscript: &CanonicalDScriptV1) -> Result<String, String> {
    let mut sorted = dscript.clone();
    sorted
        .functions
        .sort_by(|a, b| a.function_id.cmp(&b.function_id));
    let value: Value = serde_json::to_value(&sorted)
        .map_err(|error| format!("serialize CanonicalDScriptV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotations(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn sample_dscript() -> CanonicalDScriptV1 {
        CanonicalDScriptV1 {
            schema: CANONICAL_DSCRIPT_SCHEMA.to_string(),
            application_id: "app:smart-sentinel-lab".to_string(),
            dscript_id: "dscript:sensor-pipeline".to_string(),
            revision: 1,
            functions: vec![
                CanonicalDFunctionV1 {
                    function_id: "decode_uplink".to_string(),
                    annotations: annotations(&[("stateless", "true")]),
                },
                CanonicalDFunctionV1 {
                    function_id: "threshold_alert".to_string(),
                    annotations: annotations(&[("stateless", "false"), ("idempotent", "true")]),
                },
            ],
        }
    }

    #[test]
    fn valid_minimal_dscript_has_no_findings() {
        let dscript = CanonicalDScriptV1 {
            schema: CANONICAL_DSCRIPT_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_id: "dscript:x".to_string(),
            revision: 1,
            functions: vec![CanonicalDFunctionV1 {
                function_id: "solo".to_string(),
                annotations: BTreeMap::new(),
            }],
        };
        assert_eq!(dscript.validate(), Vec::new());
    }

    #[test]
    fn valid_multi_function_dscript_has_no_findings() {
        assert_eq!(sample_dscript().validate(), Vec::new());
    }

    #[test]
    fn empty_functions_collection_validates_cleanly() {
        let dscript = CanonicalDScriptV1 {
            schema: CANONICAL_DSCRIPT_SCHEMA.to_string(),
            application_id: "app:empty".to_string(),
            dscript_id: "dscript:empty".to_string(),
            revision: 1,
            functions: vec![],
        };
        assert_eq!(dscript.validate(), Vec::new());
    }

    #[test]
    fn empty_annotation_collection_is_valid() {
        let dscript = CanonicalDScriptV1 {
            schema: CANONICAL_DSCRIPT_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_id: "dscript:x".to_string(),
            revision: 1,
            functions: vec![CanonicalDFunctionV1 {
                function_id: "no_annotations".to_string(),
                annotations: BTreeMap::new(),
            }],
        };
        assert_eq!(dscript.validate(), Vec::new());
    }

    #[test]
    fn wrong_schema_rejected() {
        let mut dscript = sample_dscript();
        dscript.schema = "datum.dscript/2".to_string();
        assert!(dscript
            .validate()
            .iter()
            .any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn blank_application_id_rejected() {
        let mut dscript = sample_dscript();
        dscript.application_id = "   ".to_string();
        assert!(dscript.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn blank_dscript_id_rejected() {
        let mut dscript = sample_dscript();
        dscript.dscript_id = "".to_string();
        assert!(dscript.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn revision_zero_rejected() {
        let mut dscript = sample_dscript();
        dscript.revision = 0;
        assert!(dscript
            .validate()
            .iter()
            .any(|f| f.code == "revision_invalid"));
    }

    #[test]
    fn blank_function_id_rejected() {
        let mut dscript = sample_dscript();
        dscript.functions[0].function_id = "  ".to_string();
        assert!(dscript.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn duplicate_function_id_rejected() {
        let mut dscript = sample_dscript();
        let id = dscript.functions[0].function_id.clone();
        dscript.functions[1].function_id = id;
        assert!(dscript
            .validate()
            .iter()
            .any(|f| f.code == "duplicate_function_id"));
    }

    #[test]
    fn duplicate_function_id_hidden_by_ordering_is_still_caught() {
        let mut dscript = sample_dscript();
        let id = dscript.functions[0].function_id.clone();
        dscript.functions.push(CanonicalDFunctionV1 {
            function_id: id,
            annotations: BTreeMap::new(),
        });
        assert!(dscript
            .validate()
            .iter()
            .any(|f| f.code == "duplicate_function_id"));
    }

    #[test]
    fn blank_annotation_key_rejected() {
        let mut dscript = sample_dscript();
        dscript.functions[0]
            .annotations
            .insert("   ".to_string(), "value".to_string());
        assert!(dscript.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn round_trip_preserves_identity_and_annotations() {
        let dscript = sample_dscript();
        let json = serde_json::to_string(&dscript).unwrap();
        let round_tripped: CanonicalDScriptV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, dscript);
    }

    #[test]
    fn digest_is_deterministic_for_the_same_semantic_input() {
        let d1 = recompute_dscript_digest(&sample_dscript()).unwrap();
        let d2 = recompute_dscript_digest(&sample_dscript()).unwrap();
        assert_eq!(d1, d2);
        assert!(d1.starts_with("sha256:"));
        assert_eq!(d1.len(), 7 + 64);
    }

    #[test]
    fn digest_changes_when_function_semantics_change() {
        let a = sample_dscript();
        let mut b = sample_dscript();
        b.functions[0]
            .annotations
            .insert("new_key".to_string(), "new_value".to_string());
        assert_ne!(
            recompute_dscript_digest(&a).unwrap(),
            recompute_dscript_digest(&b).unwrap()
        );
    }

    #[test]
    fn digest_changes_when_revision_changes() {
        let a = sample_dscript();
        let mut b = sample_dscript();
        b.revision = 2;
        assert_ne!(
            recompute_dscript_digest(&a).unwrap(),
            recompute_dscript_digest(&b).unwrap()
        );
    }

    #[test]
    fn digest_is_order_independent_for_functions() {
        let mut reordered = sample_dscript();
        reordered.functions.reverse();
        assert_eq!(
            recompute_dscript_digest(&sample_dscript()).unwrap(),
            recompute_dscript_digest(&reordered).unwrap()
        );
    }

    /// Authority-negative test, matching `dserver::core::canonical_dscript`'s
    /// own equivalent test exactly.
    #[test]
    fn no_forbidden_authority_field_exists_on_the_type() {
        let value = serde_json::to_value(sample_dscript()).unwrap();
        let text = value.to_string();
        let forbidden = [
            "node_id",
            "\"node\"",
            "host",
            "placement",
            "zone",
            "target_node",
            "runtime_host",
            "image",
            "digest",
            "wasm",
            "binary",
            "artifact_path",
            "resolved_image",
            "container",
            "\"services\"",
            "\"calls\"",
            "call_id",
            "source_service_id",
            "target_service_id",
            "health",
            "readiness",
            "convergence",
            "freshness",
            "\"pid\"",
            "restart_count",
            "sequence",
            "parallel",
            "branch",
            "loop",
            "event",
            "trigger",
            "retry",
            "timeout",
            "rollback",
            "fallback",
            "recovery",
        ];
        for keyword in forbidden {
            assert!(!text.contains(keyword), "forbidden authority-adjacent key '{keyword}' found in canonical D-Script JSON: {text}");
        }
    }

    /// Real cross-crate mapper fixture (same pattern as
    /// `canonical_dgraph`'s own cross-crate test): the shared canonical-v1
    /// wire shape is IDENTICAL on both sides, so one JSON file proves parity
    /// directly. Phase 116C: the fixture is now the PURE canonical document
    /// (no sibling `x_note`/`expected_digest_sha256` metadata keys, which
    /// `deny_unknown_fields` would itself reject) -- the expected digest is
    /// a literal, matching `dserver::core::canonical_dscript`'s own test
    /// exactly.
    #[test]
    fn dserver_cross_crate_fixture_produces_the_expected_digest() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("canonical-v1")
            .join("dscript.sensor-pipeline.json");
        let text = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));

        let dscript: CanonicalDScriptV1 = serde_json::from_str(&text)
            .expect("fixture must deserialize as this crate's CanonicalDScriptV1");
        assert_eq!(
            dscript.validate(),
            Vec::new(),
            "the positive fixture must validate cleanly"
        );

        let digest = recompute_dscript_digest(&dscript).unwrap();
        assert_eq!(
            digest, "sha256:3f01cf0ee34e8e368d5f0241617588b04e894564f82aa553565e69c723ed38ae",
            "this crate's recomputed digest must match the fixture's known digest"
        );
    }

    // ---- Phase 116C: wire-contract parity for the unknown-field/         //
    // ---- duplicate-key decisions (no admission-boundary helper mirrored  //
    // ---- here -- see the module doc comment) ----------------------------//

    /// Contract parity: an unrecognised field must be rejected here exactly
    /// as it is on the dserver side, since `deny_unknown_fields` is part of
    /// the shared wire contract, not a dserver-only helper.
    #[test]
    fn unknown_field_is_rejected_matching_dserver_side_policy() {
        let json = r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[{"function_id":"decode","node_id":"fog-01","annotations":{}}]}"#;
        let result: Result<CanonicalDScriptV1, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "unknown field 'node_id' must be rejected, not silently ignored"
        );
    }

    /// Contract parity: a duplicate JSON object key for a known field is
    /// rejected by serde's derived `Deserialize` unconditionally (empirically
    /// verified; independent of `deny_unknown_fields`), matching the
    /// dserver-side behavior exactly.
    #[test]
    fn duplicate_json_object_key_for_a_known_field_is_rejected() {
        let json = r#"{"schema":"datum.dscript/1","schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[]}"#;
        let result: Result<CanonicalDScriptV1, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "a duplicate JSON key for a known field must be rejected"
        );
    }

    /// A caller-provided `dscript_digest` field must never become trusted
    /// digest authority -- with `deny_unknown_fields` active it is rejected
    /// outright, matching the dserver-side test exactly.
    #[test]
    fn caller_provided_dscript_digest_field_is_rejected_never_trusted() {
        let json = r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"dscript_digest":"sha256:deadbeef","functions":[]}"#;
        let result: Result<CanonicalDScriptV1, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
