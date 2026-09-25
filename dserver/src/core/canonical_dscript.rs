//! `datum.dscript/1` (Phase 116B) -- the canonical COMPILE-TIME source
//! representation of a D-Application (ADR-0001): a D-Script is "a set of
//! D-Functions with non-functional annotations." D-Compile (Phase 117, not
//! yet implemented) will map this into a D-Graph and D-Code, with
//! D-Function -> D-Serv cardinality that may be 1:1, 1:N, M:1, or M:N.
//!
//! Phase 116A (`documentation/reference/phase116a-dscript-architecture-audit.md`)
//! found this concept existed ONLY as documentation before this module: no
//! Rust type, parser, or compiler for DScript had ever existed on any
//! branch. The only prior concrete artifact -- a hand-authored VS1 fixture
//! sketch embedding `source.dscript_id`/`provenance.d_functions` fields
//! directly inside D-Graph/D-Serv-artifact documents -- is explicitly
//! disclaimed by ADR-0017 as never implemented, and is NOT the basis for
//! this type; `CanonicalDGraphV1` (this crate) has no such field and this
//! module does not revive it.
//!
//! **What this type deliberately does NOT do** (Phase 116A negative-space
//! findings; each is a structural absence, not an oversight):
//! - It is NOT a parser, a compiler, or a runtime -- it is the typed target
//!   a future human/tool-authored source (or, until one is proven
//!   necessary, direct JSON authoring exactly like D-Graph/D-Map today)
//!   deserializes into.
//! - It does NOT generate a D-Graph, D-Call edges, or D-Code -- that is
//!   D-Compile's job (Phase 117), not implemented here.
//! - It carries NO placement field (no node/host/zone) -- D-Map remains the
//!   sole placement authority.
//! - It carries NO executable identity (no image/digest/wasm module/binary)
//!   -- D-Code/ServiceArtifact remain that authority downstream of
//!   D-Compile.
//! - It carries NO D-Graph topology (no services/calls/edges) of its own.
//! - It carries NO observed-runtime-state field (health/readiness/
//!   convergence/PID) -- D-Monitor remains that authority.
//! - It carries NO control-flow (sequence/parallel/branch/loop/event) or
//!   failure/recovery (retry/timeout/rollback) construct: Phase 116A found
//!   no repository evidence DScript ever owned these, and Phase 115's
//!   governed reconciliation already owns operational recovery.
//! - `functions` is a Rust `Vec` purely as a serialization container for
//!   what the canonical definition calls a SET of D-Functions: array
//!   position carries no execution-order meaning, and `recompute_dscript_digest`
//!   sorts by `function_id` before hashing so reordering never changes
//!   identity (mirrors `canonical_dgraph`'s own `services`/`calls`
//!   ordering decision, ADR-0017).
//! - There is no `dscript_digest` FIELD on this type (Phase 116B explicitly
//!   defers digest/revision-acceptance authority): `recompute_dscript_digest`
//!   below is a free function, exactly mirroring
//!   `canonical_dgraph::recompute_dgraph_digest`'s own precedent of never
//!   storing a digest-of-itself on the type.
//!
//! **Non-functional annotations** (ADR-0001's own phrase): Phase 116A found
//! no repository evidence defining what these contain -- not CPU/memory/
//! QoS/latency/placement-hints/retries/timeouts/readiness-dependencies/
//! security requirements; none of those are given any special meaning here.
//! Rather than invent that semantics, or accept an unconstrained
//! `serde_json::Value`/`BTreeMap<String, Value>` catch-all that could
//! smuggle in arbitrary nested structure (placement, executable content,
//! control flow) through the back door, v1 represents only the minimum
//! structural fact that a D-Function carries zero or more NAMED, OPAQUE
//! string annotations: `BTreeMap<String, String>`. A `BTreeMap` (not
//! `HashMap`) guarantees deterministic iteration/serialization order,
//! matching this project's canonical-contract discipline; a `String` value
//! (never `Value`) cannot itself represent a nested object, an array, a
//! number, or any recursive structure -- it is structurally incapable of
//! encoding placement, executable identity, or control flow, no matter what
//! key or value text a caller supplies. This is a deliberately conservative
//! placeholder representation, not a semantic authority: nothing in this
//! module interprets any annotation key or value.
//!
//! Independently mirrored (not shared) by `DATUM::agent::canonical_dscript`,
//! per this project's established no-shared-crate discipline (ADR-0007):
//! parity is proven by a real cross-crate fixture + test on each side, never
//! by type sharing.
//!
//! **Phase 116C addition: the pure input boundary.** `decode_and_validate_dscript_json`
//! is the seam a future caller (Phase 117 DCompile, first and foremost) uses
//! to turn untrusted serialized `datum.dscript/1` JSON into a typed,
//! structurally-valid, digest-known `CanonicalDScriptV1` -- or a
//! deterministic, structured explanation of exactly why not. This grants NO
//! deployment, placement, execution, current-state, or "accepted" authority:
//! a successfully decoded-and-validated document means only "this exact
//! serialized source decoded into the canonical v1 type and passed its
//! self-contained structural contract," nothing more. This function exists
//! ONLY on the `dserver` side (see `documentation/reference/phase116c-dscript-input-admission-and-closure.md`
//! Section on mirror discipline): `DATUM::agent::canonical_dscript` proves
//! wire-contract parity (including the `deny_unknown_fields` decision below)
//! but has no proven consumer for this convenience boundary, so it is not
//! duplicated there merely for symmetry.
//!
//! **Phase 116C: `#[serde(deny_unknown_fields)]`.** Added to both types,
//! following the newer, stricter Phase-112+ canonical convention already
//! established by `canonical_dforward`/`canonical_dmap_v2`/`canonical_dmonitor`
//! (see ADR-0024's own words: "the newer, stricter Phase-112+ canonical
//! convention... `deny_unknown_fields` on every struct") rather than the
//! older Stage-110-era `canonical_dgraph`/`canonical_dmap` v1 convention.
//! `datum.dscript/1` is a brand-new v1 contract with no deployed backward-
//! compatibility burden, so the stricter, newer convention applies with no
//! migration cost. This closes a real integrity gap: without it, a caller
//! could write `{"function_id": "decode", "node_id": "fog-01"}` and a human
//! reader might wrongly believe placement was expressed, when the field was
//! actually silently ignored. With `deny_unknown_fields`, that input is a
//! hard deserialization error instead.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::jcs;

pub const CANONICAL_DSCRIPT_SCHEMA: &str = "datum.dscript/1";

/// One structured validation finding, mirroring the `{code, message}` shape
/// `canonical_dgraph::CanonicalValidationFinding` already uses. Defined
/// locally (not imported from `canonical_dgraph`) because, unlike
/// `canonical_dmap` (which validates cross-references against D-Graph/
/// D-Continuum), a D-Script's v1 validation is fully self-contained and
/// never touches another canonical contract (Section 5/9 of the Phase 116A
/// audit: no D-Serv references exist in v1).
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
    /// itself -- a project references one or more application D-Script
    /// revisions, never the reverse.
    pub application_id: String,
    pub dscript_id: String,
    pub revision: u64,
    /// A SET of D-Functions (ADR-0001's own wording), serialized as a `Vec`
    /// purely as a wire-format container -- see the module doc comment.
    /// Empty is structurally legal (mirrors `canonical_dgraph`'s own
    /// `empty_collections_validate_cleanly` precedent): whether a
    /// zero-function D-Script is USEFUL is a D-Compile-level question, not
    /// a v1 structural-validity question.
    #[serde(default)]
    pub functions: Vec<CanonicalDFunctionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDFunctionV1 {
    /// Stable identity, matching this project's `<noun>_id` convention
    /// (`service_id`, `call_id`, `dgraph_id`) rather than a display name --
    /// D-Compile, not this type, decides how a D-Function's identity maps to
    /// any eventual generated symbol or D-Serv.
    pub function_id: String,
    /// See the module doc comment: a deliberately conservative, opaque
    /// name->value string map. Never interpreted by this module.
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

/// Canonical JCS+SHA-256 digest (ADR-0006), never self-referential (this
/// type has no digest-of-itself field to omit). `functions` is a
/// canonically UNORDERED SET keyed by `function_id` (mirroring
/// `canonical_dgraph`'s explicit array-ordering decision, ADR-0017): a
/// deterministic sorted copy is hashed, so two semantically identical
/// D-Scripts that merely list their D-Functions in a different order
/// produce the IDENTICAL digest. The wire JSON itself is never reordered by
/// this function -- serialization/round-tripping preserves whatever order
/// the caller supplied; only the bytes fed into the digest are normalized.
pub fn recompute_dscript_digest(dscript: &CanonicalDScriptV1) -> Result<String, String> {
    let mut sorted = dscript.clone();
    sorted
        .functions
        .sort_by(|a, b| a.function_id.cmp(&b.function_id));
    let value: Value = serde_json::to_value(&sorted)
        .map_err(|error| format!("serialize CanonicalDScriptV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

/// Phase 116C. Why "decode_and_validate", not "admit"/"admission": this
/// repository already uses "admitted"/"admission" with an established,
/// distinct, authority-adjacent meaning (`dmonitor_interpretation.rs`'s
/// `admitted`/`admitted_evidence` -- whether an observation contributes to a
/// drift/convergence decision; `operational_dgraph.rs`'s "re-admission" --
/// a Resource Registry authority revalidation). Reusing that word here would
/// invite exactly the confusion Phase 116A/116B have been careful to avoid:
/// this function grants no authority of any kind.
#[derive(Debug)]
pub enum DScriptInputError {
    /// The input bytes could not even be decoded as `datum.dscript/1`'s
    /// typed JSON shape -- malformed JSON, a wrong-typed field, a duplicate
    /// JSON object key for a recognised field (serde's derived
    /// `Deserialize` rejects this unconditionally; see
    /// `duplicate_json_object_key_for_a_known_field_is_rejected` below), or
    /// -- because of `deny_unknown_fields` -- an unrecognised field. This is
    /// a syntactic failure: `CanonicalDScriptV1::validate()` is never
    /// reached.
    Deserialization(String),
    /// The input decoded successfully into a structurally well-typed
    /// `CanonicalDScriptV1`, but failed its own self-contained
    /// `validate()`. These are EXACTLY the findings `validate()` itself
    /// would produce -- never duplicated or reinterpreted by this function.
    Validation(Vec<CanonicalValidationFinding>),
    /// `recompute_dscript_digest` failed on an already-validated document.
    /// Given this type's field set (`String`/`u64`/`BTreeMap<String, String>`
    /// only), this is not reachable in practice -- JCS canonicalization can
    /// only fail on a non-finite float, which no field here can ever hold --
    /// but it is a real, distinct failure class, not folded into
    /// `Deserialization` or silently unwrapped.
    DigestComputation(String),
}

/// A `CanonicalDScriptV1` that has been decoded and has passed its own
/// structural `validate()`, together with its independently-recomputed
/// canonical digest. Carries NO deployment, placement, execution, or
/// current-state authority -- see the module doc comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDScriptInput {
    pub document: CanonicalDScriptV1,
    pub digest_sha256: String,
}

/// The pure input boundary: JSON text in, a typed+validated+digested
/// document or a structured reason why not, out. No I/O, no filesystem
/// access (a caller reads bytes/text itself and passes them in), no
/// network access, no persistence, no host mutation. Never trusts a
/// caller-supplied digest -- there is no `dscript_digest` field on
/// `CanonicalDScriptV1` to trust in the first place (Phase 116B decision,
/// preserved here); the digest returned is always freshly recomputed from
/// the typed document this function itself decoded.
pub fn decode_and_validate_dscript_json(
    json: &str,
) -> Result<ValidatedDScriptInput, DScriptInputError> {
    let document: CanonicalDScriptV1 = serde_json::from_str(json)
        .map_err(|error| DScriptInputError::Deserialization(error.to_string()))?;
    let findings = document.validate();
    if !findings.is_empty() {
        return Err(DScriptInputError::Validation(findings));
    }
    let digest_sha256 =
        recompute_dscript_digest(&document).map_err(DScriptInputError::DigestComputation)?;
    Ok(ValidatedDScriptInput {
        document,
        digest_sha256,
    })
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

    /// A D-Script is defined as a SET of D-Functions; an empty set is
    /// structurally legal, mirroring `canonical_dgraph`'s own
    /// `empty_collections_validate_cleanly` precedent for `services`/`calls`.
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
        // Adversarial: duplicate appended at a non-adjacent position, not
        // merely consecutive -- the checker must not rely on sortedness.
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

    /// The array-ordering decision (mirroring `canonical_dgraph`'s ADR-0017
    /// precedent): `functions` is canonically an unordered set keyed by
    /// `function_id` -- reordering the array must NOT change the digest,
    /// and array position must never be read as execution order.
    #[test]
    fn digest_is_order_independent_for_functions() {
        let mut reordered = sample_dscript();
        reordered.functions.reverse();
        assert_eq!(
            recompute_dscript_digest(&sample_dscript()).unwrap(),
            recompute_dscript_digest(&reordered).unwrap()
        );
    }

    /// Authority-negative test (Phase 116A Section 16 / this phase's own
    /// negative-space contract): serialize a representative D-Script and
    /// confirm no key ever associated with placement, executable identity,
    /// D-Graph topology, observed runtime state, control flow, or
    /// failure/recovery semantics appears anywhere in the wire JSON.
    #[test]
    fn no_forbidden_authority_field_exists_on_the_type() {
        let value = serde_json::to_value(sample_dscript()).unwrap();
        let text = value.to_string();
        let forbidden = [
            // placement (Section 14/16 of the 116B task)
            "node_id",
            "\"node\"",
            "host",
            "placement",
            "zone",
            "target_node",
            "runtime_host",
            // executable identity (Section 15)
            "image",
            "digest",
            "wasm",
            "binary",
            "artifact_path",
            "resolved_image",
            "container",
            // D-Graph topology (Section 16)
            "\"services\"",
            "\"calls\"",
            "call_id",
            "source_service_id",
            "target_service_id",
            // observed runtime state (Section 17)
            "health",
            "readiness",
            "convergence",
            "freshness",
            "\"pid\"",
            "restart_count",
            // control flow / failure-recovery (Sections 12-13)
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

    /// Real cross-crate mapper fixture (same pattern
    /// `canonical_dforward`/`canonical_dmonitor` already use once
    /// `deny_unknown_fields` is active: the fixture is the PURE canonical
    /// document with no sibling `x_note`/`expected_digest_sha256` metadata
    /// keys -- those would themselves be rejected as unknown fields -- and
    /// the expected digest is instead a literal in the test, exactly like
    /// `canonical_dforward::real_positive_fixture_digest_matches_dmap_v2_bundle_reference`).
    /// `DATUM::agent::canonical_dscript`'s own test of this exact file
    /// independently deserializes and re-hashes it; both sides must recover
    /// the SAME literal digest.
    #[test]
    fn dserver_cross_crate_fixture_produces_the_expected_digest() {
        let text = include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json"
        );
        let dscript: CanonicalDScriptV1 = serde_json::from_str(text)
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

    // ---- Phase 116C: pure input boundary --------------------------------- //

    const SENSOR_PIPELINE_JSON: &str = include_str!(
        "../../../scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json"
    );
    const EXPECTED_SENSOR_PIPELINE_DIGEST: &str =
        "sha256:3f01cf0ee34e8e368d5f0241617588b04e894564f82aa553565e69c723ed38ae";

    fn minimal_valid_json() -> String {
        r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[{"function_id":"solo","annotations":{}}]}"#.to_string()
    }

    /// I1 -- valid minimal JSON: decode succeeds, validate succeeds, typed
    /// identity preserved, canonical digest returned.
    #[test]
    fn i1_valid_minimal_json_is_decoded_validated_and_digested() {
        let result = decode_and_validate_dscript_json(&minimal_valid_json())
            .expect("must decode and validate");
        assert_eq!(result.document.dscript_id, "dscript:x");
        assert_eq!(result.document.functions.len(), 1);
        let recomputed = recompute_dscript_digest(&result.document).unwrap();
        assert_eq!(result.digest_sha256, recomputed);
    }

    /// I2 -- representative multi-function DScript succeeds.
    #[test]
    fn i2_representative_multi_function_dscript_succeeds() {
        let result = decode_and_validate_dscript_json(SENSOR_PIPELINE_JSON)
            .expect("must decode and validate");
        assert_eq!(result.document.functions.len(), 2);
        assert_eq!(result.digest_sha256, EXPECTED_SENSOR_PIPELINE_DIGEST);
    }

    /// I3 -- function array permutation: both succeed, and produce the SAME
    /// canonical digest through the full input boundary, not merely through
    /// `recompute_dscript_digest` called directly.
    #[test]
    fn i3_function_array_permutation_yields_the_same_digest_through_the_boundary() {
        let forward = decode_and_validate_dscript_json(SENSOR_PIPELINE_JSON)
            .expect("forward order must decode");
        let reversed_json = r#"{"schema":"datum.dscript/1","application_id":"app:smart-sentinel-lab","dscript_id":"dscript:sensor-pipeline","revision":1,"functions":[{"function_id":"threshold_alert","annotations":{"idempotent":"true","stateless":"false"}},{"function_id":"decode_uplink","annotations":{"stateless":"true"}}]}"#;
        let reversed =
            decode_and_validate_dscript_json(reversed_json).expect("reversed order must decode");
        assert_eq!(forward.digest_sha256, reversed.digest_sha256);
        assert_ne!(
            forward.document, reversed.document,
            "the typed documents differ in field order but the digest must not"
        );
    }

    /// I4 -- annotations are preserved exactly and contribute to a
    /// deterministic digest through the boundary.
    #[test]
    fn i4_annotations_preserved_exactly_with_deterministic_digest() {
        let result = decode_and_validate_dscript_json(SENSOR_PIPELINE_JSON).expect("must decode");
        let decode_uplink = result
            .document
            .functions
            .iter()
            .find(|function| function.function_id == "decode_uplink")
            .expect("decode_uplink must be present");
        assert_eq!(
            decode_uplink.annotations.get("stateless"),
            Some(&"true".to_string())
        );
        let again =
            decode_and_validate_dscript_json(SENSOR_PIPELINE_JSON).expect("must decode again");
        assert_eq!(result.digest_sha256, again.digest_sha256);
    }

    /// I5 -- the existing shared canonical fixture passes through the new
    /// input boundary and retains its known digest (same assertion as I2,
    /// stated separately to match the task's own required-test numbering).
    #[test]
    fn i5_shared_fixture_passes_through_the_input_boundary_with_its_known_digest() {
        let result = decode_and_validate_dscript_json(SENSOR_PIPELINE_JSON)
            .expect("fixture must pass through the boundary");
        assert_eq!(result.digest_sha256, EXPECTED_SENSOR_PIPELINE_DIGEST);
    }

    /// I6 -- malformed JSON is a deserialization failure, not a validation
    /// failure -- `CanonicalDScriptV1::validate()` is never reached.
    #[test]
    fn i6_malformed_json_is_a_deserialization_failure() {
        let error = decode_and_validate_dscript_json("{ not valid json").unwrap_err();
        assert!(matches!(error, DScriptInputError::Deserialization(_)));
    }

    /// I7 -- valid JSON, wrong schema: this decodes fine (schema is just a
    /// `String` field) but fails structural validation with `schema_invalid`.
    #[test]
    fn i7_wrong_schema_is_a_validation_failure_with_schema_invalid() {
        let json = minimal_valid_json().replace("datum.dscript/1", "datum.dscript/2");
        let error = decode_and_validate_dscript_json(&json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(
                    findings.iter().any(|f| f.code == "schema_invalid"),
                    "{findings:?}"
                );
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I8 -- blank application_id -> missing_field.
    #[test]
    fn i8_blank_application_id_is_missing_field() {
        let json = minimal_valid_json().replace("\"app:x\"", "\"   \"");
        let error = decode_and_validate_dscript_json(&json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(findings.iter().any(|f| f.code == "missing_field"))
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I9 -- blank dscript_id -> missing_field.
    #[test]
    fn i9_blank_dscript_id_is_missing_field() {
        let json = minimal_valid_json().replace("\"dscript:x\"", "\"\"");
        let error = decode_and_validate_dscript_json(&json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(findings.iter().any(|f| f.code == "missing_field"))
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I10 -- revision 0 -> revision_invalid.
    #[test]
    fn i10_revision_zero_is_revision_invalid() {
        let json = minimal_valid_json().replace("\"revision\":1", "\"revision\":0");
        let error = decode_and_validate_dscript_json(&json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(findings.iter().any(|f| f.code == "revision_invalid"))
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I11 -- blank function_id -> missing_field.
    #[test]
    fn i11_blank_function_id_is_missing_field() {
        let json = minimal_valid_json().replace("\"solo\"", "\"   \"");
        let error = decode_and_validate_dscript_json(&json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(findings.iter().any(|f| f.code == "missing_field"))
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I12 -- duplicate function_id -> duplicate_function_id.
    #[test]
    fn i12_duplicate_function_id_is_rejected() {
        let json = r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[{"function_id":"same","annotations":{}},{"function_id":"same","annotations":{}}]}"#;
        let error = decode_and_validate_dscript_json(json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(findings.iter().any(|f| f.code == "duplicate_function_id"))
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I13 -- blank annotation key -> missing_field.
    #[test]
    fn i13_blank_annotation_key_is_missing_field() {
        let json = r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[{"function_id":"solo","annotations":{"   ":"value"}}]}"#;
        let error = decode_and_validate_dscript_json(json).unwrap_err();
        match error {
            DScriptInputError::Validation(findings) => {
                assert!(findings.iter().any(|f| f.code == "missing_field"))
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// I14 -- an unrecognised field is REJECTED (the explicit policy this
    /// phase adopted, Section 9): `deny_unknown_fields` turns it into a
    /// deserialization failure, never a silently-ignored no-op.
    #[test]
    fn i14_unknown_field_is_rejected_not_silently_ignored() {
        let json = r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[{"function_id":"decode","node_id":"fog-01","annotations":{}}]}"#;
        let error = decode_and_validate_dscript_json(json).unwrap_err();
        assert!(
            matches!(error, DScriptInputError::Deserialization(_)),
            "unknown field 'node_id' must be rejected, not ignored: {error:?}"
        );
    }

    /// Caller-provided-digest negative case (Section 21): an unsupported
    /// `dscript_digest` field must never become trusted digest authority --
    /// with `deny_unknown_fields` active it is rejected outright, exactly
    /// like any other unrecognised field.
    #[test]
    fn caller_provided_dscript_digest_field_is_rejected_never_trusted() {
        let json = r#"{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"dscript_digest":"sha256:deadbeef","functions":[]}"#;
        let error = decode_and_validate_dscript_json(json).unwrap_err();
        assert!(matches!(error, DScriptInputError::Deserialization(_)));
    }

    /// Positive proof that the digest actually used is always recomputed,
    /// never trusted from any external source: two byte-different but
    /// semantically identical JSON documents (compact vs. pretty-printed --
    /// re-serialized via `serde_json::Value` so no string field content is
    /// ever touched) decoded independently must yield the same
    /// freshly-recomputed digest.
    #[test]
    fn digest_returned_by_the_boundary_is_always_freshly_recomputed() {
        let a = decode_and_validate_dscript_json(&minimal_valid_json()).unwrap();
        let value: Value = serde_json::from_str(&minimal_valid_json()).unwrap();
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        let b = decode_and_validate_dscript_json(&pretty).unwrap();
        assert_eq!(a.digest_sha256, b.digest_sha256);
    }

    /// Documents the empirically-verified serde_json behavior (Section 10):
    /// a duplicate JSON object key for a KNOWN field is unconditionally
    /// rejected by serde's derived `Deserialize` impl -- independent of
    /// `deny_unknown_fields`, which only governs UNRECOGNISED keys. No
    /// bespoke duplicate-key handling was written because none is needed.
    #[test]
    fn duplicate_json_object_key_for_a_known_field_is_rejected() {
        let json = r#"{"schema":"datum.dscript/1","schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[]}"#;
        let error = decode_and_validate_dscript_json(json).unwrap_err();
        match error {
            DScriptInputError::Deserialization(message) => {
                assert!(message.contains("duplicate field"), "{message}")
            }
            other => panic!("expected Deserialization, got {other:?}"),
        }
    }
}
