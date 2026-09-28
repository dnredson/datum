//! `datum.dcompile-request/1` (Phase 117B) -- the explicit, typed
//! compile-time input a future D-Compile lowering step (Phase 117C) needs
//! beyond `ValidatedDScriptInput` alone. This module contains NO lowering:
//! no `compile()`, no `lower()`, no `generate_dgraph()`. It exists only so
//! 117C has trustworthy, explicit, pre-validated inputs instead of inventing
//! defaults.
//!
//! Phase 117A (`documentation/reference/phase117a-dcompile-architecture-lowering-audit.md`)
//! found that `datum.dscript/1` alone cannot supply: which D-Functions group
//! into which D-Serv, what identity a candidate D-Graph should have, or what
//! resource requirement each generated D-Serv declares. None of those may be
//! invented (no `function_id == service_id` convention, no resource
//! defaults, no reading resource/grouping data out of DScript's
//! deliberately opaque annotations -- ADR-0025). This type is exactly that
//! missing explicit input, nothing more.
//!
//! **V1 mapping semantics: EXACT PARTITION, not the vocabulary's full M:N.**
//! Phase 117A found concrete historical evidence (the disclaimed VS1 sketch)
//! only for a DISJOINT partition: one D-Script, several D-Servs, each owning
//! a distinct, non-overlapping subset of D-Functions. Whether a D-Function
//! may legitimately belong to MULTIPLE D-Servs simultaneously (shared vs.
//! duplicated implementation) is explicitly UNDEFINED by any repository
//! evidence (117A Section 21). `datum.dcompile-request/1` v1 therefore
//! REQUIRES every D-Script function to be mapped to EXACTLY ONE service --
//! not zero, not more than one. This is a v1 slice limitation, not a claim
//! that the DATUM reference model's eventual overlapping 1:N/M:N case is
//! impossible; a later schema revision may extend this once real
//! implementation-sharing/duplication semantics are defined by evidence.
//!
//! **D-Node ABI is NOT a field on this type.** Phase 117A established
//! `DNODE_ABI_V0` ("datum-dnode/0") is the sole legal D-Node ABI anywhere in
//! this architecture today -- there is no second value for a caller to
//! meaningfully choose between. Exposing an ABI selector here would be fake
//! configurability; Phase 117C assigns the sole valid constant explicitly
//! when it eventually lowers a D-Graph, exactly as every existing
//! hand-authored D-Graph fixture already does.
//!
//! **No D-Call input, no D-Code content.** Phase 117A found DScript carries
//! no call/invocation relationship and this repository has no source-to-binary
//! build path at all -- D-Code content is always externally built and
//! already digested before this repository ever sees it. Both remain
//! entirely out of this type's scope; a future, separate contract revision
//! or sibling type would need to add them once real evidence exists.
//!
//! **Wire-contract parity is NOT mirrored to `DATUM::agent`.** Phase 116C
//! established that DCompile is expected to be a server-side/control-plane
//! responsibility, and Phase 117A found no D-Node-agent consumer for
//! compile-time input. This type therefore exists only in `dserver`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::canonical_dgraph::CanonicalDServResourceRequirementV1;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::canonical_dscript::{CanonicalValidationFinding, ValidatedDScriptInput};
use crate::core::jcs;

pub const CANONICAL_DCOMPILE_REQUEST_SCHEMA: &str = "datum.dcompile-request/1";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

/// `dscript_ref`/`output_dgraph_id`/`output_dgraph_revision` reuse
/// `canonical_dmap::CanonicalContractReferenceV1` and this project's plain
/// identity-field convention respectively -- NOT a new DCompile->D-Map
/// coupling. `CanonicalContractReferenceV1` is already a genuinely
/// project-wide `{id, revision, digest}` reference shape, reused by
/// `dcall.rs`, `dcode_authorization.rs`, `ddeploy.rs`, and
/// `operational_dgraph.rs` for unrelated purposes -- importing it here
/// follows that exact, already-established precedent rather than creating
/// one. `output_dgraph_id`/`output_dgraph_revision` are explicit, flat
/// fields (Phase 117A Section 5's recommendation): a future D-Graph's
/// identity must not become a hidden, undocumented parameter Phase 117C
/// invents on its own -- "same validated D-Script + same validated compile
/// request -> same candidate D-Graph" requires that identity to already be
/// part of this request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileRequestV1 {
    pub schema: String,
    /// Must exactly match the D-Script's own `application_id`
    /// (`validate_against_dscript`) -- never inferred or rewritten.
    pub application_id: String,
    /// Binds this request immutably to the EXACT D-Script it was prepared
    /// for: `id` = `dscript_id`, plus `revision` and the D-Script's own
    /// canonical digest. A caller merely asserting "this belongs to that
    /// D-Script" is never trusted -- `validate_against_dscript` recomputes
    /// the comparison against a real `ValidatedDScriptInput`.
    pub dscript_ref: CanonicalContractReferenceV1,
    pub output_dgraph_id: String,
    pub output_dgraph_revision: u64,
    /// The complete D-Function -> D-Serv partition. Order carries no
    /// meaning (mirrors D-Script's own `functions` set semantics);
    /// `recompute_dcompile_request_digest` normalizes order before hashing.
    #[serde(default)]
    pub services: Vec<DCompileServiceMappingV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileServiceMappingV1 {
    /// The mapping supplier chooses this identity in v1 -- DCompile does
    /// NOT invent, hash, or derive `service_id` from `function_ids` (Phase
    /// 117A Section 7: no precedent anywhere in this codebase for hashing a
    /// canonical identity out of content; every existing identity is
    /// human/tool-chosen text).
    pub service_id: String,
    /// Non-empty (Phase 117B Section 11: a service with zero functions has
    /// no evidentiary meaning; an entirely function-less D-Script uses an
    /// empty `services` list instead of a zero-function service entry).
    #[serde(default)]
    pub function_ids: Vec<String>,
    /// Explicit, required, never defaulted (Phase 117A Section 12: no
    /// resource default exists anywhere in this codebase) and never read
    /// from D-Script annotations (ADR-0025) -- a declared compile-time
    /// requirement, not an observation.
    pub resource_requirement: CanonicalDServResourceRequirementV1,
}

impl DCompileRequestV1 {
    /// Pure, self-contained structural validation -- never touches a
    /// D-Script or any other contract. Collects every finding; returns
    /// `Vec::new()` iff valid. Overlapping `function_ids` membership BETWEEN
    /// two `services` entries is a purely internal request invariant (no
    /// D-Script needed to detect it) and is checked here, not in
    /// `validate_against_dscript`.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DCOMPILE_REQUEST_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!(
                    "schema must be '{CANONICAL_DCOMPILE_REQUEST_SCHEMA}', got '{}'",
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
        if self.dscript_ref.id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "dscript_ref.id must be a non-empty string",
            ));
        }
        if self.dscript_ref.revision < 1 {
            findings.push(finding(
                "revision_invalid",
                "dscript_ref.revision must be an integer >= 1",
            ));
        }
        if !jcs::is_canonical_digest_format(&self.dscript_ref.digest) {
            findings.push(finding(
                "dscript_digest_malformed",
                format!(
                    "dscript_ref.digest must be 'sha256:<64 lowercase hex>', got '{}'",
                    self.dscript_ref.digest
                ),
            ));
        }
        if self.output_dgraph_id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "output_dgraph_id must be a non-empty string",
            ));
        }
        if self.output_dgraph_revision < 1 {
            findings.push(finding(
                "revision_invalid",
                "output_dgraph_revision must be an integer >= 1",
            ));
        }

        let mut seen_service_ids = std::collections::HashSet::new();
        let mut function_owner: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::new();
        for service in &self.services {
            if service.service_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "service_id must be a non-empty string",
                ));
            } else if !seen_service_ids.insert(service.service_id.as_str()) {
                findings.push(finding(
                    "duplicate_service_id",
                    format!("duplicate service_id '{}'", service.service_id),
                ));
            }
            if service.function_ids.is_empty() {
                findings.push(finding(
                    "empty_service_function_set",
                    format!("service '{}' declares zero function_ids; a service must own at least one function", service.service_id),
                ));
            }
            let mut seen_within_service = std::collections::HashSet::new();
            for function_id in &service.function_ids {
                if function_id.trim().is_empty() {
                    findings.push(finding(
                        "missing_field",
                        "function_id must be a non-empty string",
                    ));
                    continue;
                }
                if !seen_within_service.insert(function_id.as_str()) {
                    findings.push(finding(
                        "duplicate_function_id",
                        format!(
                            "service '{}' lists function_id '{function_id}' more than once",
                            service.service_id
                        ),
                    ));
                    continue;
                }
                if let Some(other_service) =
                    function_owner.insert(function_id.as_str(), service.service_id.as_str())
                {
                    findings.push(finding(
                        "overlapping_function_mapping",
                        format!(
                            "function_id '{function_id}' is mapped to both '{other_service}' and '{}' -- v1 requires an exact partition (Phase 117A: overlapping D-Function membership is undefined semantics, not yet supported)",
                            service.service_id
                        ),
                    ));
                }
            }
            if service.resource_requirement.cpu_cores < 1 {
                findings.push(finding(
                    "invalid_cpu_requirement",
                    format!(
                        "service '{}' resource_requirement.cpu_cores must be >= 1, got {}",
                        service.service_id, service.resource_requirement.cpu_cores
                    ),
                ));
            }
            if service.resource_requirement.memory_bytes < 1 {
                findings.push(finding(
                    "invalid_memory_requirement",
                    format!(
                        "service '{}' resource_requirement.memory_bytes must be >= 1, got {}",
                        service.service_id, service.resource_requirement.memory_bytes
                    ),
                ));
            }
        }

        findings
    }

    /// Relational validation against a REAL `ValidatedDScriptInput` -- never
    /// duplicates `CanonicalDScriptV1::validate()`'s own structural rules,
    /// and never mutates either side to fix a mistake (no auto-adding a
    /// missing function, no silently discarding an unknown one). Callers
    /// should call `validate()` first; this function assumes the request is
    /// already internally well-formed and focuses purely on the
    /// request<->D-Script relationship.
    pub fn validate_against_dscript(
        &self,
        dscript: &ValidatedDScriptInput,
    ) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.application_id != dscript.document.application_id {
            findings.push(finding(
                "application_id_mismatch",
                format!(
                    "request application_id '{}' does not match dscript application_id '{}'",
                    self.application_id, dscript.document.application_id
                ),
            ));
        }
        if self.dscript_ref.id != dscript.document.dscript_id {
            findings.push(finding(
                "dscript_id_mismatch",
                format!(
                    "dscript_ref.id '{}' does not match dscript_id '{}'",
                    self.dscript_ref.id, dscript.document.dscript_id
                ),
            ));
        }
        if self.dscript_ref.revision != dscript.document.revision {
            findings.push(finding(
                "dscript_revision_mismatch",
                format!(
                    "dscript_ref.revision {} does not match dscript revision {}",
                    self.dscript_ref.revision, dscript.document.revision
                ),
            ));
        }
        if self.dscript_ref.digest != dscript.digest_sha256 {
            findings.push(finding(
                "dscript_digest_mismatch",
                format!(
                    "dscript_ref.digest '{}' does not match the dscript's actual digest '{}'",
                    self.dscript_ref.digest, dscript.digest_sha256
                ),
            ));
        }

        let dscript_functions: BTreeSet<&str> = dscript
            .document
            .functions
            .iter()
            .map(|f| f.function_id.as_str())
            .collect();
        let mapped_functions: BTreeSet<&str> = self
            .services
            .iter()
            .flat_map(|service| service.function_ids.iter().map(String::as_str))
            .collect();

        for function_id in mapped_functions.difference(&dscript_functions) {
            findings.push(finding(
                "unknown_function_mapping",
                format!("mapping references function_id '{function_id}', which does not exist in the D-Script"),
            ));
        }
        for function_id in dscript_functions.difference(&mapped_functions) {
            findings.push(finding(
                "unmapped_function",
                format!("D-Script function_id '{function_id}' is not mapped to any service"),
            ));
        }

        findings
    }
}

/// Canonical JCS+SHA-256 digest (ADR-0006), never self-referential (this
/// type has no digest-of-itself field). `services` and each service's
/// `function_ids` are canonically UNORDERED SETS: a deterministic sorted
/// copy is hashed, so reordering either never changes the digest. The wire
/// JSON itself is never reordered -- only the bytes fed into the digest are
/// normalized. This digest is evidence/provenance only; it grants no
/// authority.
pub fn recompute_dcompile_request_digest(request: &DCompileRequestV1) -> Result<String, String> {
    let mut sorted = request.clone();
    for service in &mut sorted.services {
        service.function_ids.sort();
    }
    sorted
        .services
        .sort_by(|a, b| a.service_id.cmp(&b.service_id));
    let value: Value = serde_json::to_value(&sorted)
        .map_err(|error| format!("serialize DCompileRequestV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

/// Mirrors `canonical_dscript::DScriptInputError` exactly (same three
/// classes, same rationale): a syntactic decode failure never reaches
/// `validate()`; a structural validation failure carries the ACTUAL
/// findings; digest computation failure is modeled honestly rather than
/// unwrapped, even though unreachable in practice for this type's field set.
#[derive(Debug)]
pub enum DCompileRequestInputError {
    Deserialization(String),
    Validation(Vec<CanonicalValidationFinding>),
    DigestComputation(String),
}

/// A `DCompileRequestV1` that has decoded and passed its own STRUCTURAL
/// `validate()` -- nothing more. This does NOT confirm the request
/// corresponds to any particular D-Script, that its mapping is complete, or
/// that its mapping is correct: call `DCompileRequestV1::validate_against_dscript`
/// with a real `ValidatedDScriptInput` for that (Phase 117B Section 26: a
/// structurally-valid request must never be conflated with a relationally-valid
/// one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedDCompileRequest {
    pub request: DCompileRequestV1,
    pub digest_sha256: String,
}

/// Phase 117J: the typed half of the input boundary -- an already-typed
/// `DCompileRequestV1` in (no JSON text, no re-serialization), a
/// structurally-valid typed request + its recomputed digest, or a
/// structured reason why not, out. `decode_and_validate_dcompile_request_json`
/// below is now a thin JSON-decode wrapper around this exact function, so
/// both the historical JSON-string boundary and any future typed caller
/// (Phase 117J's production submission endpoint, which constructs this type
/// in-memory with a server-derived `dscript_ref` rather than decoding it
/// from caller-supplied JSON) share ONE semantic validation/digest
/// implementation. No cross-validation against any D-Script here (that step
/// is explicit and separate -- see `validate_against_dscript`).
pub fn validate_and_digest_dcompile_request(
    request: DCompileRequestV1,
) -> Result<ValidatedDCompileRequest, DCompileRequestInputError> {
    let findings = request.validate();
    if !findings.is_empty() {
        return Err(DCompileRequestInputError::Validation(findings));
    }
    let digest_sha256 = recompute_dcompile_request_digest(&request)
        .map_err(DCompileRequestInputError::DigestComputation)?;
    Ok(ValidatedDCompileRequest {
        request,
        digest_sha256,
    })
}

/// The pure input boundary for `datum.dcompile-request/1`, mirroring
/// `canonical_dscript::decode_and_validate_dscript_json` exactly: JSON text
/// in, a structurally-valid typed request + its recomputed digest, or a
/// structured reason why not, out. Deserialization is the only step owned
/// here -- everything after that is delegated to
/// `validate_and_digest_dcompile_request`.
pub fn decode_and_validate_dcompile_request_json(
    json: &str,
) -> Result<ValidatedDCompileRequest, DCompileRequestInputError> {
    let request: DCompileRequestV1 = serde_json::from_str(json)
        .map_err(|error| DCompileRequestInputError::Deserialization(error.to_string()))?;
    validate_and_digest_dcompile_request(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dscript::{
        decode_and_validate_dscript_json, CanonicalDFunctionV1, CanonicalDScriptV1,
    };

    const SENSOR_PIPELINE_DSCRIPT_JSON: &str = include_str!(
        "../../../scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json"
    );
    const SENSOR_PIPELINE_REQUEST_JSON: &str = include_str!(
        "../../../scripts/fixtures/datum-contracts/canonical-v1/dcompile-request.sensor-pipeline.json"
    );

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 {
            cpu_cores,
            memory_bytes,
        }
    }

    fn dscript_functions(ids: &[&str]) -> CanonicalDScriptV1 {
        CanonicalDScriptV1 {
            schema: "datum.dscript/1".to_string(),
            application_id: "app:x".to_string(),
            dscript_id: "dscript:x".to_string(),
            revision: 1,
            functions: ids
                .iter()
                .map(|id| CanonicalDFunctionV1 {
                    function_id: id.to_string(),
                    annotations: Default::default(),
                })
                .collect(),
        }
    }

    fn validated(dscript: CanonicalDScriptV1) -> ValidatedDScriptInput {
        let digest_sha256 =
            crate::core::canonical_dscript::recompute_dscript_digest(&dscript).unwrap();
        ValidatedDScriptInput {
            document: dscript,
            digest_sha256,
        }
    }

    fn dscript_ref_for(dscript: &ValidatedDScriptInput) -> CanonicalContractReferenceV1 {
        CanonicalContractReferenceV1 {
            id: dscript.document.dscript_id.clone(),
            revision: dscript.document.revision,
            digest: dscript.digest_sha256.clone(),
        }
    }

    fn sample_request() -> DCompileRequestV1 {
        let dscript = validated(dscript_functions(&["A", "B"]));
        DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_ref: dscript_ref_for(&dscript),
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![
                DCompileServiceMappingV1 {
                    service_id: "svc-a".to_string(),
                    function_ids: vec!["A".to_string()],
                    resource_requirement: res(1, 134_217_728),
                },
                DCompileServiceMappingV1 {
                    service_id: "svc-b".to_string(),
                    function_ids: vec!["B".to_string()],
                    resource_requirement: res(1, 67_108_864),
                },
            ],
        }
    }

    // ---- B1-B3: positive grouping shapes ---------------------------------- //

    #[test]
    fn b1_one_function_to_one_service_is_valid() {
        let dscript = validated(dscript_functions(&["A"]));
        let request = DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_ref: dscript_ref_for(&dscript),
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![DCompileServiceMappingV1 {
                service_id: "svc-a".to_string(),
                function_ids: vec!["A".to_string()],
                resource_requirement: res(1, 134_217_728),
            }],
        };
        assert_eq!(request.validate(), Vec::new());
        assert_eq!(request.validate_against_dscript(&dscript), Vec::new());
    }

    #[test]
    fn b2_multiple_functions_to_one_service_is_valid_m_to_1() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let request = DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_ref: dscript_ref_for(&dscript),
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![DCompileServiceMappingV1 {
                service_id: "svc-combined".to_string(),
                function_ids: vec!["A".to_string(), "B".to_string()],
                resource_requirement: res(2, 268_435_456),
            }],
        };
        assert_eq!(request.validate(), Vec::new());
        assert_eq!(request.validate_against_dscript(&dscript), Vec::new());
    }

    #[test]
    fn b3_functions_partitioned_across_multiple_services_is_valid() {
        let request = sample_request();
        let dscript = validated(dscript_functions(&["A", "B"]));
        assert_eq!(request.validate(), Vec::new());
        assert_eq!(request.validate_against_dscript(&dscript), Vec::new());
    }

    // ---- B4-B5: digest order independence ---------------------------------- //

    #[test]
    fn b4_service_order_does_not_affect_digest() {
        let mut reordered = sample_request();
        reordered.services.reverse();
        assert_eq!(
            recompute_dcompile_request_digest(&sample_request()).unwrap(),
            recompute_dcompile_request_digest(&reordered).unwrap()
        );
    }

    #[test]
    fn b5_function_id_order_within_a_service_does_not_affect_digest() {
        let mut request = sample_request();
        request.services[0].function_ids = vec!["X".to_string(), "Y".to_string()];
        let mut reordered = request.clone();
        reordered.services[0].function_ids.reverse();
        assert_eq!(
            recompute_dcompile_request_digest(&request).unwrap(),
            recompute_dcompile_request_digest(&reordered).unwrap()
        );
    }

    // ---- B6: empty DScript + empty mapping --------------------------------- //

    #[test]
    fn b6_empty_dscript_and_empty_mapping_is_valid() {
        let dscript = validated(dscript_functions(&[]));
        let request = DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_ref: dscript_ref_for(&dscript),
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![],
        };
        assert_eq!(request.validate(), Vec::new());
        assert_eq!(request.validate_against_dscript(&dscript), Vec::new());
    }

    #[test]
    fn empty_dscript_with_nonempty_mapping_fails() {
        let dscript = validated(dscript_functions(&[]));
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        request.application_id = "app:x".to_string();
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings
                .iter()
                .any(|f| f.code == "unknown_function_mapping"),
            "{findings:?}"
        );
    }

    #[test]
    fn nonempty_dscript_with_empty_mapping_fails() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let request = DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_ref: dscript_ref_for(&dscript),
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![],
        };
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings
                .iter()
                .filter(|f| f.code == "unmapped_function")
                .count()
                == 2,
            "{findings:?}"
        );
    }

    // ---- B7-B8: cross-validation + resource preservation ------------------- //

    #[test]
    fn b7_exact_dscript_identity_and_digest_match_validates() {
        let request = sample_request();
        let dscript = validated(dscript_functions(&["A", "B"]));
        assert_eq!(request.validate_against_dscript(&dscript), Vec::new());
    }

    #[test]
    fn b8_resource_requirements_preserved_exactly_through_round_trip() {
        let request = sample_request();
        let json = serde_json::to_string(&request).unwrap();
        let round_tripped: DCompileRequestV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, request);
        assert_eq!(
            round_tripped.services[0].resource_requirement,
            res(1, 134_217_728)
        );
    }

    // ---- B9-B15: structural negatives --------------------------------------- //

    #[test]
    fn b9_wrong_schema_rejected() {
        let mut request = sample_request();
        request.schema = "datum.dcompile-request/2".to_string();
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn b10_blank_application_id_rejected() {
        let mut request = sample_request();
        request.application_id = "   ".to_string();
        assert!(request.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn b11_blank_dscript_reference_id_rejected() {
        let mut request = sample_request();
        request.dscript_ref.id = "".to_string();
        assert!(request.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn b12_dscript_reference_revision_zero_rejected() {
        let mut request = sample_request();
        request.dscript_ref.revision = 0;
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "revision_invalid"));
    }

    #[test]
    fn b13_malformed_dscript_digest_rejected() {
        let mut request = sample_request();
        request.dscript_ref.digest = "not-a-digest".to_string();
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "dscript_digest_malformed"));
    }

    #[test]
    fn b14_blank_output_dgraph_id_rejected() {
        let mut request = sample_request();
        request.output_dgraph_id = "".to_string();
        assert!(request.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn b15_output_dgraph_revision_zero_rejected() {
        let mut request = sample_request();
        request.output_dgraph_revision = 0;
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "revision_invalid"));
    }

    // ---- B16-B22: mapping structural negatives ------------------------------ //

    #[test]
    fn b16_duplicate_service_id_rejected() {
        let mut request = sample_request();
        request.services[1].service_id = request.services[0].service_id.clone();
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "duplicate_service_id"));
    }

    #[test]
    fn b17_blank_service_id_rejected() {
        let mut request = sample_request();
        request.services[0].service_id = "  ".to_string();
        assert!(request.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn b18_service_with_zero_function_ids_rejected() {
        let mut request = sample_request();
        request.services[0].function_ids = vec![];
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "empty_service_function_set"));
    }

    #[test]
    fn b19_blank_function_id_rejected() {
        let mut request = sample_request();
        request.services[0].function_ids = vec!["   ".to_string()];
        assert!(request.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn b20_duplicate_function_id_within_one_service_rejected() {
        let mut request = sample_request();
        request.services[0].function_ids = vec!["A".to_string(), "A".to_string()];
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "duplicate_function_id"));
    }

    /// B21 / Section 35: the important mapping test -- proves this contract
    /// does NOT support undefined overlapping many-to-many semantics. This
    /// is a v1 partition-semantics limitation, NOT a claim that the DATUM
    /// reference model's eventual M:N capability is impossible.
    #[test]
    fn b21_overlapping_function_across_two_services_rejected() {
        let mut request = sample_request();
        request.services[1].function_ids = vec!["A".to_string()]; // A now in both svc-a and svc-b
        let findings = request.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "overlapping_function_mapping"),
            "{findings:?}"
        );
    }

    #[test]
    fn b22_unknown_function_rejected() {
        let dscript = validated(dscript_functions(&["A"]));
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        request.services = vec![DCompileServiceMappingV1 {
            service_id: "svc-x".to_string(),
            function_ids: vec!["A".to_string(), "Z".to_string()],
            resource_requirement: res(1, 1),
        }];
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings
                .iter()
                .any(|f| f.code == "unknown_function_mapping"),
            "{findings:?}"
        );
    }

    // ---- B23: coverage ------------------------------------------------------ //

    #[test]
    fn b23_unmapped_dscript_function_rejected() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let request = DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dscript_ref: dscript_ref_for(&dscript),
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![DCompileServiceMappingV1 {
                service_id: "svc-a".to_string(),
                function_ids: vec!["A".to_string()],
                resource_requirement: res(1, 1),
            }],
        };
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings
                .iter()
                .any(|f| f.code == "unmapped_function" && f.message.contains('B')),
            "{findings:?}"
        );
    }

    // ---- B24-B27: cross-validation identity mismatches ---------------------- //

    #[test]
    fn b24_application_id_mismatch_rejected() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        request.application_id = "app:different".to_string();
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings.iter().any(|f| f.code == "application_id_mismatch"),
            "{findings:?}"
        );
    }

    #[test]
    fn b25_dscript_id_mismatch_rejected() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        request.dscript_ref.id = "dscript:someone-else".to_string();
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings.iter().any(|f| f.code == "dscript_id_mismatch"),
            "{findings:?}"
        );
    }

    #[test]
    fn b26_dscript_revision_mismatch_rejected() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        request.dscript_ref.revision = 99;
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings
                .iter()
                .any(|f| f.code == "dscript_revision_mismatch"),
            "{findings:?}"
        );
    }

    #[test]
    fn b27_dscript_digest_mismatch_rejected() {
        let dscript = validated(dscript_functions(&["A", "B"]));
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        request.dscript_ref.digest = format!("sha256:{}", "0".repeat(64));
        let findings = request.validate_against_dscript(&dscript);
        assert!(
            findings.iter().any(|f| f.code == "dscript_digest_mismatch"),
            "{findings:?}"
        );
    }

    // ---- B28-B29: resource requirement negatives ---------------------------- //

    #[test]
    fn b28_cpu_cores_zero_rejected() {
        let mut request = sample_request();
        request.services[0].resource_requirement.cpu_cores = 0;
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "invalid_cpu_requirement"));
    }

    #[test]
    fn b29_memory_bytes_zero_rejected() {
        let mut request = sample_request();
        request.services[0].resource_requirement.memory_bytes = 0;
        assert!(request
            .validate()
            .iter()
            .any(|f| f.code == "invalid_memory_requirement"));
    }

    // ---- B30-B31: unknown field / caller digest, through the JSON boundary - //

    #[test]
    fn b30_unknown_json_field_is_rejected() {
        let json = r#"{"schema":"datum.dcompile-request/1","application_id":"app:x","dscript_ref":{"id":"dscript:x","revision":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"output_dgraph_id":"dgraph:x","output_dgraph_revision":1,"node_id":"fog-01","services":[]}"#;
        let error = decode_and_validate_dcompile_request_json(json).unwrap_err();
        assert!(
            matches!(error, DCompileRequestInputError::Deserialization(_)),
            "unknown field 'node_id' must be rejected: {error:?}"
        );
    }

    #[test]
    fn b31_caller_supplied_request_digest_field_is_rejected() {
        let json = r#"{"schema":"datum.dcompile-request/1","application_id":"app:x","dscript_ref":{"id":"dscript:x","revision":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"output_dgraph_id":"dgraph:x","output_dgraph_revision":1,"request_digest":"sha256:deadbeef","services":[]}"#;
        let error = decode_and_validate_dcompile_request_json(json).unwrap_err();
        assert!(matches!(
            error,
            DCompileRequestInputError::Deserialization(_)
        ));
    }

    #[test]
    fn speculative_calls_field_is_rejected() {
        let json = r#"{"schema":"datum.dcompile-request/1","application_id":"app:x","dscript_ref":{"id":"dscript:x","revision":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"output_dgraph_id":"dgraph:x","output_dgraph_revision":1,"calls":[],"services":[]}"#;
        let error = decode_and_validate_dcompile_request_json(json).unwrap_err();
        assert!(matches!(
            error,
            DCompileRequestInputError::Deserialization(_)
        ));
    }

    #[test]
    fn speculative_retry_field_is_rejected() {
        let json = r#"{"schema":"datum.dcompile-request/1","application_id":"app:x","dscript_ref":{"id":"dscript:x","revision":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"output_dgraph_id":"dgraph:x","output_dgraph_revision":1,"retry":3,"services":[]}"#;
        let error = decode_and_validate_dcompile_request_json(json).unwrap_err();
        assert!(matches!(
            error,
            DCompileRequestInputError::Deserialization(_)
        ));
    }

    // ---- annotations are never interpreted ---------------------------------- //

    #[test]
    fn annotations_never_influence_validation_outcome() {
        // A DScript whose annotations look suggestive of resource/grouping
        // intent must have zero effect on request validation -- the request
        // above never even reads annotations, this proves it by using
        // deliberately misleading annotation content and confirming the
        // SAME request validates identically regardless.
        let mut dscript_doc = dscript_functions(&["A", "B"]);
        dscript_doc.functions[0]
            .annotations
            .insert("cpu".to_string(), "16".to_string());
        dscript_doc.functions[0]
            .annotations
            .insert("node".to_string(), "fog-99".to_string());
        dscript_doc.functions[1]
            .annotations
            .insert("group".to_string(), "svc-b".to_string());
        let dscript = validated(dscript_doc);
        let mut request = sample_request();
        request.dscript_ref = dscript_ref_for(&dscript);
        assert_eq!(request.validate(), Vec::new());
        assert_eq!(request.validate_against_dscript(&dscript), Vec::new());
    }

    // ---- authority-negative -------------------------------------------------- //

    #[test]
    fn no_forbidden_authority_field_exists_on_the_type() {
        let value = serde_json::to_value(sample_request()).unwrap();
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
            "wasm",
            "binary",
            "artifact_path",
            "resolved_image",
            "container",
            "dcode",
            "content_id",
            "signature_ref",
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
            "dnode_abi",
        ];
        for keyword in forbidden {
            assert!(!text.contains(keyword), "forbidden authority-adjacent key '{keyword}' found in canonical DCompile request JSON: {text}");
        }
    }

    // ---- Phase 117J Section 9: typed/JSON boundary equivalence -------------- //

    #[test]
    fn typed_and_json_boundaries_agree_on_a_valid_request() {
        let via_json =
            decode_and_validate_dcompile_request_json(SENSOR_PIPELINE_REQUEST_JSON).unwrap();
        let typed: DCompileRequestV1 =
            serde_json::from_str(SENSOR_PIPELINE_REQUEST_JSON).unwrap();
        let via_typed = validate_and_digest_dcompile_request(typed).unwrap();
        assert_eq!(via_json, via_typed);
    }

    #[test]
    fn typed_and_json_boundaries_agree_on_the_same_validation_findings() {
        let mut bad = sample_request();
        bad.output_dgraph_revision = 0;
        let via_typed = validate_and_digest_dcompile_request(bad.clone()).unwrap_err();
        let bad_json = serde_json::to_string(&bad).unwrap();
        let via_json = decode_and_validate_dcompile_request_json(&bad_json).unwrap_err();
        match (via_json, via_typed) {
            (DCompileRequestInputError::Validation(a), DCompileRequestInputError::Validation(b)) => {
                assert_eq!(a, b)
            }
            other => panic!("expected both to be Validation with equal findings: {other:?}"),
        }
    }

    // ---- fixture: shared DScript + matching DCompile request ---------------- //

    #[test]
    fn shared_fixture_request_matches_shared_fixture_dscript() {
        let dscript = decode_and_validate_dscript_json(SENSOR_PIPELINE_DSCRIPT_JSON)
            .expect("dscript fixture must decode and validate");
        let validated_request =
            decode_and_validate_dcompile_request_json(SENSOR_PIPELINE_REQUEST_JSON)
                .expect("request fixture must decode and validate structurally");
        assert_eq!(
            validated_request.request.validate_against_dscript(&dscript),
            Vec::new()
        );
    }

    #[test]
    fn shared_fixture_request_digest_is_deterministic() {
        let a = decode_and_validate_dcompile_request_json(SENSOR_PIPELINE_REQUEST_JSON).unwrap();
        let b = decode_and_validate_dcompile_request_json(SENSOR_PIPELINE_REQUEST_JSON).unwrap();
        assert_eq!(a.digest_sha256, b.digest_sha256);
        assert!(a.digest_sha256.starts_with("sha256:"));
    }

    #[test]
    fn digest_changes_when_a_semantically_meaningful_field_changes() {
        let base = sample_request();
        let base_digest = recompute_dcompile_request_digest(&base).unwrap();

        let mut dscript_changed = base.clone();
        dscript_changed.dscript_ref.digest = format!("sha256:{}", "1".repeat(64));
        assert_ne!(
            recompute_dcompile_request_digest(&dscript_changed).unwrap(),
            base_digest
        );

        let mut dgraph_id_changed = base.clone();
        dgraph_id_changed.output_dgraph_id = "dgraph:other".to_string();
        assert_ne!(
            recompute_dcompile_request_digest(&dgraph_id_changed).unwrap(),
            base_digest
        );

        let mut service_id_changed = base.clone();
        service_id_changed.services[0].service_id = "svc-renamed".to_string();
        assert_ne!(
            recompute_dcompile_request_digest(&service_id_changed).unwrap(),
            base_digest
        );

        let mut membership_changed = base.clone();
        membership_changed.services[0]
            .function_ids
            .push("extra".to_string());
        assert_ne!(
            recompute_dcompile_request_digest(&membership_changed).unwrap(),
            base_digest
        );

        let mut cpu_changed = base.clone();
        cpu_changed.services[0].resource_requirement.cpu_cores += 1;
        assert_ne!(
            recompute_dcompile_request_digest(&cpu_changed).unwrap(),
            base_digest
        );

        let mut memory_changed = base.clone();
        memory_changed.services[0].resource_requirement.memory_bytes += 1;
        assert_ne!(
            recompute_dcompile_request_digest(&memory_changed).unwrap(),
            base_digest
        );
    }
}
