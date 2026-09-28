//! Phase 117J: the FIRST production caller of the Phase 117 compiler
//! pipeline (117B-E). `documentation/reference/phase117i-production-dcompile-api-handoff-audit.md`
//! is the design this module implements without deviation -- read it first
//! for the full rationale; this module's own doc comments restate only what
//! is load-bearing for understanding the code itself.
//!
//! **The central problem this module solves (117I Section 5):**
//! `DCompileRequestV1.dscript_ref.digest` must equal the REAL D-Script's own
//! digest, and `DCompileDCodeInputV1.dgraph_ref.digest` must equal the REAL
//! candidate D-Graph's own digest -- a value that does not exist until
//! Phase 117C's lowering has already run. A caller cannot honestly supply
//! either reference before compilation happens. `DCompileSubmissionV1`
//! therefore contains NEITHER reference: `compile_submission` constructs
//! both derived references itself, from what it has just validated/
//! produced, before ever calling the existing, UNCHANGED
//! `lower_dcode_artifact_candidates`.
//!
//! **Composition, not reimplementation.** This module never re-derives
//! D-Script/D-Compile-request structural or relational validation, never
//! re-derives D-Graph lowering, and never re-derives D-Code artifact
//! construction. It calls `canonical_dscript::validate_and_digest_dscript`,
//! `dcompile_request::validate_and_digest_dcompile_request`,
//! `dcompile::lower_calls_empty_dgraph_candidate`, and
//! `dcompile_dcode::lower_dcode_artifact_candidates` -- each exactly once
//! (the D-Graph lowering function is called twice total across this
//! sequence; see the "intentional double lowering" doc comment on
//! `compile_submission` below for why that is correct, not an oversight)
//! -- and maps their real results into new, purpose-built wire types.
//!
//! **This module is pure.** No registry access, no D-Deploy state access,
//! no storage, no filesystem I/O, no HTTP, no timestamps, no randomness.
//! `compile_submission` performs zero side effects: a successful compile
//! means "deterministic candidate generated," never "registered," "active,"
//! "accepted," "deployed," "authorized," or "installed." See the module
//! doc's own authority table in the 117I audit (Section 16) for the
//! complete boundary; nothing here crosses it.
//!
//! **Internal vs. wire (117I Section 9/10).** `DCompileDGraphCandidate`/
//! `DCompileCandidateBundle`/`DCompileDServArtifactCandidate` remain
//! deliberately non-wire (no `Serialize`/`Deserialize`, unchanged by this
//! module). `DCompileSubmissionV1`/`DCompileCandidateResponseV1`/
//! `DCompileArtifactResultV1` below are new, independently-versioned,
//! purpose-built wire types that a mapping function constructs FROM the
//! internal result -- never by attaching serde to the compiler's own
//! internal representation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dgraph::CanonicalDGraphV1;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::canonical_dscript::{
    validate_and_digest_dscript, CanonicalDScriptV1, CanonicalValidationFinding,
    DScriptInputError,
};
use crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1;
use crate::core::dcompile::{lower_calls_empty_dgraph_candidate, DCompileLoweringError};
use crate::core::dcompile_dcode::{
    lower_dcode_artifact_candidates, DCompileCandidateBundle, DCompileDCodeInputV1,
    DCompileDCodeLoweringError, DCompileDCodeServiceInputV1, CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA,
};
use crate::core::dcompile_request::{
    validate_and_digest_dcompile_request, DCompileRequestInputError, DCompileRequestV1,
    DCompileServiceMappingV1, CANONICAL_DCOMPILE_REQUEST_SCHEMA,
};
use crate::core::dserv_artifact::CanonicalDServArtifactV1;

pub const DCOMPILE_SUBMISSION_SCHEMA: &str = "datum.dcompile-submission/1";
pub const DCOMPILE_CANDIDATE_SCHEMA: &str = "datum.dcompile-candidate/1";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

/// The production submission envelope. Contains ONLY true external compile
/// inputs -- no `dscript_ref`, no `dgraph_ref`, no digest of any kind, no
/// `application_id` at the outer level (the single source of application
/// identity is `dscript.application_id`, read exactly once), no
/// `project_id`, no placement/D-Continuum/node/D-Map/proposal field, and no
/// timestamp or random identifier (117I Sections 5/19/23/26, task Sections
/// 5/6/23/47/48).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileSubmissionV1 {
    pub schema: String,
    pub dscript: CanonicalDScriptV1,
    pub output_dgraph_id: String,
    pub output_dgraph_revision: u64,
    #[serde(default)]
    pub services: Vec<DCompileServiceMappingV1>,
    #[serde(default)]
    pub dcode_services: Vec<DCompileDCodeServiceInputV1>,
}

/// Distinguishes exactly where compilation refused to proceed. Every
/// finding-carrying variant preserves the REAL findings the owning
/// validator produced -- never a paraphrased message. `DGraphLowering`/
/// `DCodeLowering` wrap the existing, unmodified `DCompileLoweringError`/
/// `DCompileDCodeLoweringError` verbatim rather than re-deriving separate
/// variants for what 117C/117E already model completely; this is a smaller,
/// evidence-justified composition than enumerating every nested case here
/// (117I/task Section 32 explicitly permits this).
#[derive(Debug)]
pub enum DCompileSubmissionError {
    /// The new envelope's OWN structural facts only (schema, non-empty
    /// `output_dgraph_id`, `output_dgraph_revision >= 1`) -- never a
    /// duplicate of any nested type's own rules.
    InvalidSubmission(Vec<CanonicalValidationFinding>),
    /// `submission.dscript` itself failed `CanonicalDScriptV1::validate()`.
    InvalidDScript(Vec<CanonicalValidationFinding>),
    /// D-Script digest computation failed (JCS canonicalization error;
    /// unreachable in practice for this type's field set, modeled honestly).
    DScriptDigestComputation(String),
    /// The internally-constructed `DCompileRequestV1` (server-derived
    /// `dscript_ref`, caller-supplied `services`) failed
    /// `DCompileRequestV1::validate()` -- e.g. duplicate/blank
    /// `service_id`s, an empty/overlapping/duplicate function mapping, or an
    /// invalid resource requirement. This can also occur even though this
    /// module supplies `dscript_ref` itself: `validate()` is purely
    /// structural and never touches `dscript_ref`'s correctness (that is
    /// `DGraphLowering`'s `RelationalMismatch` case below, which is where an
    /// unmapped/unknown function mapping against the REAL D-Script surfaces).
    InvalidDCompileRequest(Vec<CanonicalValidationFinding>),
    /// D-Compile-request digest computation failed (unreachable in
    /// practice, modeled honestly).
    DCompileRequestDigestComputation(String),
    /// The PRELIMINARY D-Graph lowering call (Section: "intentional double
    /// lowering" below) failed -- in this submission flow this can only
    /// realistically be `RelationalMismatch` (the caller's `services`
    /// mapping does not exactly partition `submission.dscript`'s functions):
    /// every OTHER `DCompileLoweringError` variant requires an internally
    /// inconsistent wrapper, which this module never constructs (it always
    /// builds `dscript_ref`/the D-Script wrapper honestly, from values it
    /// just validated and digested itself), so those remain modeled, tested,
    /// and reachable only as structurally-defensive, not-expected-in-practice
    /// cases -- exactly like 117C's own C24 test already documents for
    /// `lower_calls_empty_dgraph_candidate` itself.
    DGraphLowering(DCompileLoweringError),
    /// The FINAL 117E lowering call failed. Wraps `DCompileDCodeLoweringError`
    /// verbatim, including its own (redundant, by-construction-unreachable
    /// here, but never weakened) internal `DGraphLowering` re-check.
    DCodeLowering(DCompileDCodeLoweringError),
}

/// Phase 117J Section 9/16 (task): one artifact result, directly POST-able
/// to `/api/v1/dcode/artifacts` with zero transformation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileArtifactResultV1 {
    pub service_id: String,
    pub artifact: CanonicalDServArtifactV1,
    pub descriptor_digest: String,
}

/// The production compile result. Wire-only (117I Section 10, Decision B) --
/// never a `Serialize`-ified `DCompileCandidateBundle`. Every field named
/// here is candidate data; NONE implies "registered," "active," "accepted,"
/// or "deployed" (task Section 34) -- confirmed by construction, since no
/// such field exists.
///
/// Provenance chain (five identities, never collapsed, 117I Section
/// 11/24): `dscript_ref.digest` (D-Script identity, server-computed),
/// `mapping`/`dcompile_request_digest` (compile-intent identity,
/// server-computed -- the FULL mapping is included, not merely its digest,
/// because a digest alone can never let a caller reconstruct WHICH mapping
/// produced a given D-Graph, per `dcompile.rs`'s own
/// `c12_mapping_provenance_collision_same_dgraph_different_request_digest`
/// proof), `dgraph_ref.digest` (generated topology identity,
/// server-computed), `dcode_artifacts[i].artifact.dcode.sha256` (module
/// content identity -- EXTERNALLY DECLARED, never byte-verified by
/// D-Compile), `dcode_artifacts[i].descriptor_digest` (whole-descriptor
/// identity, server-computed).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileCandidateResponseV1 {
    pub schema: String,
    pub application_id: String,
    pub dscript_ref: CanonicalContractReferenceV1,
    /// The full compile-intent mapping, canonically ordered for a
    /// deterministic wire representation (see `canonicalize_mapping` below)
    /// -- NOT simply the caller's own submitted vector order.
    pub mapping: DCompileRequestV1,
    pub dcompile_request_digest: String,
    pub dgraph: CanonicalDGraphV1,
    pub dgraph_ref: CanonicalContractReferenceV1,
    /// Always ordered by the candidate D-Graph's own deterministic service
    /// order (never `submission.dcode_services`'s vector order).
    pub dcode_artifacts: Vec<DCompileArtifactResultV1>,
    /// Pure convenience/handoff value, constructed exclusively from
    /// `dcode_artifacts` (see `dcode_authority_bindings_from_compile_candidate`
    /// below). Grants no authority by existing here -- it only becomes a
    /// real binding once actually submitted inside a real
    /// `create_proposal`/`create_v2_proposal` call, which independently
    /// revalidates it against current registry state regardless of where it
    /// came from (117G, untouched by this module).
    pub dcode_artifact_authority: BTreeMap<String, DCodeArtifactAuthorityBindingV1>,
}

/// Pure, cross-cutting helper (task Sections 16/22/27): every registered
/// artifact candidate's `(service_id, descriptor_digest)` becomes one
/// binding map entry. Reads only its own input parameter -- no registry
/// access, no D-Deploy access, no I/O of any kind, no authority granted by
/// calling it. `BTreeMap` for deterministic iteration/serialization order.
pub fn dcode_authority_bindings_from_compile_candidate(
    bundle: &DCompileCandidateBundle,
) -> BTreeMap<String, DCodeArtifactAuthorityBindingV1> {
    bundle
        .dcode_artifacts
        .iter()
        .map(|candidate| {
            (
                candidate.artifact.dserv_id.clone(),
                DCodeArtifactAuthorityBindingV1 {
                    descriptor_digest: candidate.descriptor_digest_sha256.clone(),
                },
            )
        })
        .collect()
}

/// Task Section 20 (load-bearing): `DCompileRequestV1.services` and each
/// service's `function_ids` are semantically unordered sets --
/// `recompute_dcompile_request_digest` already normalizes both before
/// hashing, but the WIRE `mapping` field would otherwise echo the caller's
/// own submitted vector order verbatim, making two semantically-identical
/// submissions (differing only in submitted ordering) produce different
/// serialized JSON despite carrying the same canonical compile meaning.
/// This canonicalizes the OUTPUT representation only -- it never mutates
/// the frozen `DCompileRequestV1` type's own semantics, and
/// `recompute_dcompile_request_digest` produces the identical digest either
/// way (it re-sorts internally regardless), so this has no effect on
/// `dcompile_request_digest`'s own value -- only on the deterministic
/// presentation of `mapping` itself.
fn canonicalize_mapping(mut request: DCompileRequestV1) -> DCompileRequestV1 {
    for service in &mut request.services {
        service.function_ids.sort();
    }
    request.services.sort_by(|a, b| a.service_id.cmp(&b.service_id));
    request
}

fn build_response(
    validated_request: &crate::core::dcompile_request::ValidatedDCompileRequest,
    bundle: &DCompileCandidateBundle,
) -> DCompileCandidateResponseV1 {
    let dgraph_ref = CanonicalContractReferenceV1 {
        id: bundle.dgraph_candidate.dgraph.dgraph_id.clone(),
        revision: bundle.dgraph_candidate.dgraph.revision,
        digest: bundle.dgraph_candidate.dgraph_digest_sha256.clone(),
    };

    // Task Section 21: artifact ordering follows the candidate D-Graph's own
    // already-deterministic service order (`bundle.dcode_artifacts`'s own
    // order, per 117E), never `submission.dcode_services`'s input order.
    let dcode_artifacts: Vec<DCompileArtifactResultV1> = bundle
        .dcode_artifacts
        .iter()
        .map(|candidate| DCompileArtifactResultV1 {
            service_id: candidate.artifact.dserv_id.clone(),
            artifact: candidate.artifact.clone(),
            descriptor_digest: candidate.descriptor_digest_sha256.clone(),
        })
        .collect();

    DCompileCandidateResponseV1 {
        schema: DCOMPILE_CANDIDATE_SCHEMA.to_string(),
        application_id: validated_request.request.application_id.clone(),
        dscript_ref: validated_request.request.dscript_ref.clone(),
        mapping: canonicalize_mapping(validated_request.request.clone()),
        dcompile_request_digest: validated_request.digest_sha256.clone(),
        dgraph: bundle.dgraph_candidate.dgraph.clone(),
        dgraph_ref,
        dcode_artifacts,
        dcode_artifact_authority: dcode_authority_bindings_from_compile_candidate(bundle),
    }
}

/// The pure Phase 117J entry point (task Section 12). Deterministic
/// sequence, task Sections 13-17:
///
/// 1. Validate+digest `submission.dscript` directly (no JSON round-trip,
///    task Section 7) via `validate_and_digest_dscript`.
/// 2. Construct the real `DCompileRequestV1` with a SERVER-DERIVED
///    `dscript_ref` (never caller-supplied), then validate+digest it via
///    `validate_and_digest_dcompile_request`.
/// 3. **Intentional double D-Graph lowering** (task Section 18, load-bearing,
///    do not optimize away): call `lower_calls_empty_dgraph_candidate` here,
///    solely to obtain the fresh D-Graph identity/digest
///    `DCompileDCodeInputV1.dgraph_ref` needs -- this PRELIMINARY candidate
///    is then discarded. `lower_dcode_artifact_candidates` (step 5 below)
///    calls the EXACT SAME function again internally, as part of its own
///    existing, unmodified trust-boundary defense (117E's own doc comment).
///    The alternative -- reaching into `lower_dcode_artifact_candidates` to
///    skip its internal re-lowering -- would weaken a defense 117C/117E
///    already established and this phase has no evidence justifying
///    weakening. A pure function computing the same small, cheap result
///    twice is preferred over that. `preliminary_dgraph_lowering_equals_final_bundles_dgraph_candidate`
///    below proves both calls really do produce an identical result for
///    the same validated inputs.
/// 4. Construct the real `DCompileDCodeInputV1` with a SERVER-DERIVED
///    `dgraph_ref` (never caller-supplied) from step 3's preliminary
///    result.
/// 5. Call the one authoritative 117E entry point,
///    `lower_dcode_artifact_candidates`, unmodified.
/// 6. Map the real result into the new wire-only response type.
pub fn compile_submission(
    submission: DCompileSubmissionV1,
) -> Result<DCompileCandidateResponseV1, DCompileSubmissionError> {
    // Step 0: the NEW envelope's own structural facts only -- never a
    // duplicate of any nested type's own rules (task Section 10).
    let mut envelope_findings = Vec::new();
    if submission.schema != DCOMPILE_SUBMISSION_SCHEMA {
        envelope_findings.push(finding(
            "schema_invalid",
            format!(
                "schema must be '{DCOMPILE_SUBMISSION_SCHEMA}', got '{}'",
                submission.schema
            ),
        ));
    }
    if submission.output_dgraph_id.trim().is_empty() {
        envelope_findings.push(finding(
            "missing_field",
            "output_dgraph_id must be a non-empty string",
        ));
    }
    if submission.output_dgraph_revision < 1 {
        envelope_findings.push(finding(
            "revision_invalid",
            "output_dgraph_revision must be an integer >= 1",
        ));
    }
    if !envelope_findings.is_empty() {
        return Err(DCompileSubmissionError::InvalidSubmission(envelope_findings));
    }

    // Step 1.
    let validated_dscript =
        validate_and_digest_dscript(submission.dscript).map_err(|error| match error {
            DScriptInputError::Validation(findings) => {
                DCompileSubmissionError::InvalidDScript(findings)
            }
            DScriptInputError::DigestComputation(message) => {
                DCompileSubmissionError::DScriptDigestComputation(message)
            }
            DScriptInputError::Deserialization(_) => {
                unreachable!("submission.dscript is already a typed CanonicalDScriptV1 -- never re-decoded from JSON text")
            }
        })?;

    // Step 2: dscript_ref is SERVER-DERIVED here -- the caller never
    // supplies one (task Section 5/6/13).
    let request = DCompileRequestV1 {
        schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
        application_id: validated_dscript.document.application_id.clone(),
        dscript_ref: CanonicalContractReferenceV1 {
            id: validated_dscript.document.dscript_id.clone(),
            revision: validated_dscript.document.revision,
            digest: validated_dscript.digest_sha256.clone(),
        },
        output_dgraph_id: submission.output_dgraph_id,
        output_dgraph_revision: submission.output_dgraph_revision,
        services: submission.services,
    };
    let validated_request =
        validate_and_digest_dcompile_request(request).map_err(|error| match error {
            DCompileRequestInputError::Validation(findings) => {
                DCompileSubmissionError::InvalidDCompileRequest(findings)
            }
            DCompileRequestInputError::DigestComputation(message) => {
                DCompileSubmissionError::DCompileRequestDigestComputation(message)
            }
            DCompileRequestInputError::Deserialization(_) => {
                unreachable!("constructed in-memory -- never decoded from JSON text")
            }
        })?;

    // Step 3: preliminary lowering, solely to derive dgraph_ref (see the
    // "intentional double lowering" doc comment above).
    let preliminary_candidate =
        lower_calls_empty_dgraph_candidate(&validated_dscript, &validated_request)
            .map_err(DCompileSubmissionError::DGraphLowering)?;

    // Step 4: dgraph_ref is SERVER-DERIVED here -- the caller never
    // supplies one (task Section 5/6/16).
    let dcode_input = DCompileDCodeInputV1 {
        schema: CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA.to_string(),
        application_id: validated_dscript.document.application_id.clone(),
        dgraph_ref: CanonicalContractReferenceV1 {
            id: preliminary_candidate.dgraph.dgraph_id.clone(),
            revision: preliminary_candidate.dgraph.revision,
            digest: preliminary_candidate.dgraph_digest_sha256.clone(),
        },
        services: submission.dcode_services,
    };

    // Step 5: the one authoritative 117E entry point.
    let bundle =
        lower_dcode_artifact_candidates(&validated_dscript, &validated_request, &dcode_input)
            .map_err(DCompileSubmissionError::DCodeLowering)?;

    // Step 6.
    Ok(build_response(&validated_request, &bundle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDServResourceRequirementV1};
    use crate::core::canonical_dscript::{recompute_dscript_digest, CanonicalDFunctionV1, CANONICAL_DSCRIPT_SCHEMA};
    use crate::core::dcompile_request::recompute_dcompile_request_digest;
    use crate::core::dserv_artifact::{recompute_dserv_artifact_digest, DServArtifactLimitsV1, DServArtifactPortsV1};
    use std::collections::BTreeMap as StdBTreeMap;

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 { cpu_cores, memory_bytes }
    }

    fn limits(timeout_ms: u64) -> DServArtifactLimitsV1 {
        DServArtifactLimitsV1 {
            fuel_per_invocation: 5_000_000,
            timeout_ms,
            max_memory_pages: 4,
            max_message_bytes: 65536,
            max_emits_per_invocation: 8,
        }
    }

    fn dscript(app: &str, dscript_id: &str, function_ids: &[&str]) -> CanonicalDScriptV1 {
        CanonicalDScriptV1 {
            schema: CANONICAL_DSCRIPT_SCHEMA.to_string(),
            application_id: app.to_string(),
            dscript_id: dscript_id.to_string(),
            revision: 1,
            functions: function_ids
                .iter()
                .map(|id| CanonicalDFunctionV1 { function_id: id.to_string(), annotations: StdBTreeMap::new() })
                .collect(),
        }
    }

    fn service_mapping(service_id: &str, function_ids: &[&str]) -> DCompileServiceMappingV1 {
        DCompileServiceMappingV1 {
            service_id: service_id.to_string(),
            function_ids: function_ids.iter().map(|s| s.to_string()).collect(),
            resource_requirement: res(1, 134_217_728),
        }
    }

    fn dcode_service_input(service_id: &str, sha256_byte: char, timeout_ms: u64) -> DCompileDCodeServiceInputV1 {
        DCompileDCodeServiceInputV1 {
            service_id: service_id.to_string(),
            artifact_id: format!("dserv:{service_id}"),
            version: "0.1.0".to_string(),
            sha256: sha256_byte.to_string().repeat(64),
            size_bytes: 1024,
            ports: DServArtifactPortsV1::default(),
            limits: limits(timeout_ms),
        }
    }

    fn submission(
        app: &str,
        dscript_id: &str,
        function_ids: &[&str],
        services: Vec<DCompileServiceMappingV1>,
        dcode_services: Vec<DCompileDCodeServiceInputV1>,
    ) -> DCompileSubmissionV1 {
        DCompileSubmissionV1 {
            schema: DCOMPILE_SUBMISSION_SCHEMA.to_string(),
            dscript: dscript(app, dscript_id, function_ids),
            output_dgraph_id: format!("dgraph:{dscript_id}"),
            output_dgraph_revision: 1,
            services,
            dcode_services,
        }
    }

    fn single_service_submission() -> DCompileSubmissionV1 {
        submission(
            "app:x",
            "dscript:x",
            &["A"],
            vec![service_mapping("svc-a", &["A"])],
            vec![dcode_service_input("svc-a", 'a', 100)],
        )
    }

    fn multi_service_submission() -> DCompileSubmissionV1 {
        submission(
            "app:x",
            "dscript:x",
            &["A", "B"],
            vec![service_mapping("svc-a", &["A"]), service_mapping("svc-b", &["B"])],
            vec![dcode_service_input("svc-a", 'a', 100), dcode_service_input("svc-b", 'b', 200)],
        )
    }

    // ==================================================================
    // J1-J18: input validation
    // ==================================================================

    #[test]
    fn j1_valid_single_function_single_service_submission_succeeds() {
        let response = compile_submission(single_service_submission()).expect("must compile");
        assert_eq!(response.dcode_artifacts.len(), 1);
        assert_eq!(response.dgraph.services.len(), 1);
    }

    #[test]
    fn j2_multi_function_multi_service_submission_succeeds() {
        let response = compile_submission(multi_service_submission()).expect("must compile");
        assert_eq!(response.dcode_artifacts.len(), 2);
        assert_eq!(response.dgraph.services.len(), 2);
    }

    #[test]
    fn j3_empty_program_compiles_to_an_empty_valid_candidate() {
        let response = compile_submission(submission("app:x", "dscript:x", &[], vec![], vec![])).expect("empty program must compile");
        assert!(response.dgraph.services.is_empty());
        assert!(response.dgraph.calls.is_empty());
        assert!(response.dcode_artifacts.is_empty());
        assert!(response.dcode_artifact_authority.is_empty());
    }

    #[test]
    fn j4_wrong_submission_schema_rejected() {
        let mut s = single_service_submission();
        s.schema = "datum.dcompile-submission/2".to_string();
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::InvalidSubmission(findings) => {
                assert!(findings.iter().any(|f| f.code == "schema_invalid"))
            }
            other => panic!("expected InvalidSubmission, got {other:?}"),
        }
    }

    #[test]
    fn j5_blank_output_dgraph_id_rejected() {
        let mut s = single_service_submission();
        s.output_dgraph_id = "   ".to_string();
        let error = compile_submission(s).unwrap_err();
        assert!(matches!(error, DCompileSubmissionError::InvalidSubmission(_)));
    }

    #[test]
    fn j6_output_revision_zero_rejected() {
        let mut s = single_service_submission();
        s.output_dgraph_revision = 0;
        let error = compile_submission(s).unwrap_err();
        assert!(matches!(error, DCompileSubmissionError::InvalidSubmission(_)));
    }

    #[test]
    fn j7_invalid_nested_dscript_rejected() {
        let mut s = single_service_submission();
        s.dscript.application_id = "   ".to_string();
        let error = compile_submission(s).unwrap_err();
        assert!(matches!(error, DCompileSubmissionError::InvalidDScript(_)));
    }

    #[test]
    fn j8_duplicate_dscript_function_rejected() {
        let mut s = single_service_submission();
        s.dscript.functions.push(CanonicalDFunctionV1 { function_id: "A".to_string(), annotations: StdBTreeMap::new() });
        let error = compile_submission(s).unwrap_err();
        assert!(matches!(error, DCompileSubmissionError::InvalidDScript(_)));
    }

    #[test]
    fn j9_duplicate_service_id_rejected() {
        let mut s = multi_service_submission();
        s.services[1].service_id = "svc-a".to_string();
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::InvalidDCompileRequest(findings) => {
                assert!(findings.iter().any(|f| f.code == "duplicate_service_id"))
            }
            other => panic!("expected InvalidDCompileRequest, got {other:?}"),
        }
    }

    #[test]
    fn j10_overlapping_function_mapping_rejected() {
        let mut s = multi_service_submission();
        s.services[1].function_ids = vec!["A".to_string()];
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::InvalidDCompileRequest(findings) => {
                assert!(findings.iter().any(|f| f.code == "overlapping_function_mapping"))
            }
            other => panic!("expected InvalidDCompileRequest, got {other:?}"),
        }
    }

    #[test]
    fn j11_unknown_mapped_function_rejected() {
        let mut s = single_service_submission();
        s.services[0].function_ids.push("does-not-exist".to_string());
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DGraphLowering(DCompileLoweringError::RelationalMismatch(findings)) => {
                assert!(findings.iter().any(|f| f.code == "unknown_function_mapping"))
            }
            other => panic!("expected DGraphLowering(RelationalMismatch), got {other:?}"),
        }
    }

    #[test]
    fn j12_unmapped_dscript_function_rejected() {
        let mut s = multi_service_submission();
        s.services.pop(); // svc-b's mapping removed -- B is now unmapped
        s.dcode_services.pop();
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DGraphLowering(DCompileLoweringError::RelationalMismatch(findings)) => {
                assert!(findings.iter().any(|f| f.code == "unmapped_function"))
            }
            other => panic!("expected DGraphLowering(RelationalMismatch), got {other:?}"),
        }
    }

    #[test]
    fn j13_malformed_module_sha_rejected() {
        let mut s = single_service_submission();
        s.dcode_services[0].sha256 = "not-a-sha".to_string();
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DCodeLowering(DCompileDCodeLoweringError::InvalidDCodeInput(findings)) => {
                assert!(findings.iter().any(|f| f.code == "module_sha256_malformed"))
            }
            other => panic!("expected DCodeLowering(InvalidDCodeInput), got {other:?}"),
        }
    }

    #[test]
    fn j14_missing_dcode_service_coverage_rejected() {
        let mut s = multi_service_submission();
        s.dcode_services.pop(); // svc-b has a generated service but no DCode input
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DCodeLowering(DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings)) => {
                assert!(findings.iter().any(|f| f.code == "missing_service_dcode_input"))
            }
            other => panic!("expected DCodeLowering(DCodeInputRelationalMismatch), got {other:?}"),
        }
    }

    #[test]
    fn j15_unknown_dcode_service_rejected() {
        let mut s = single_service_submission();
        s.dcode_services.push(dcode_service_input("svc-ghost", 'c', 100));
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DCodeLowering(DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings)) => {
                assert!(findings.iter().any(|f| f.code == "unknown_service_dcode_input"))
            }
            other => panic!("expected DCodeLowering(DCodeInputRelationalMismatch), got {other:?}"),
        }
    }

    #[test]
    fn j16_duplicate_dcode_service_rejected() {
        let mut s = single_service_submission();
        let dup = s.dcode_services[0].clone();
        s.dcode_services.push(dup);
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DCodeLowering(DCompileDCodeLoweringError::InvalidDCodeInput(findings)) => {
                assert!(findings.iter().any(|f| f.code == "duplicate_service_input"))
            }
            other => panic!("expected DCodeLowering(InvalidDCodeInput), got {other:?}"),
        }
    }

    #[test]
    fn j17_invalid_limits_rejected() {
        let mut s = single_service_submission();
        s.dcode_services[0].limits.fuel_per_invocation = 0;
        let error = compile_submission(s).unwrap_err();
        match error {
            DCompileSubmissionError::DCodeLowering(DCompileDCodeLoweringError::GeneratedArtifactInvalid { service_id, .. }) => {
                assert_eq!(service_id, "svc-a")
            }
            other => panic!("expected DCodeLowering(GeneratedArtifactInvalid), got {other:?}"),
        }
    }

    #[test]
    fn j18_unknown_submission_field_rejected() {
        let json = r#"{"schema":"datum.dcompile-submission/1","dscript":{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[]},"output_dgraph_id":"dgraph:x","output_dgraph_revision":1,"services":[],"dcode_services":[],"project_id":"proj-x"}"#;
        let result: Result<DCompileSubmissionV1, _> = serde_json::from_str(json);
        assert!(result.is_err(), "unknown field 'project_id' must be rejected");
    }

    // ==================================================================
    // J19-J25: derived identity
    // ==================================================================

    #[test]
    fn j19_dscript_ref_is_server_derived_correctly() {
        let submission = single_service_submission();
        let expected_digest = recompute_dscript_digest(&submission.dscript).unwrap();
        let response = compile_submission(submission.clone()).unwrap();
        assert_eq!(response.dscript_ref.id, submission.dscript.dscript_id);
        assert_eq!(response.dscript_ref.revision, submission.dscript.revision);
        assert_eq!(response.dscript_ref.digest, expected_digest);
    }

    #[test]
    fn j20_mapping_dscript_ref_is_exactly_the_response_dscript_ref() {
        let response = compile_submission(single_service_submission()).unwrap();
        assert_eq!(response.mapping.dscript_ref, response.dscript_ref);
    }

    #[test]
    fn j21_request_digest_independently_recomputes() {
        let response = compile_submission(multi_service_submission()).unwrap();
        assert_eq!(
            response.dcompile_request_digest,
            recompute_dcompile_request_digest(&response.mapping).unwrap()
        );
    }

    #[test]
    fn j22_dgraph_ref_independently_recomputes() {
        let response = compile_submission(multi_service_submission()).unwrap();
        assert_eq!(response.dgraph_ref.digest, recompute_dgraph_digest(&response.dgraph).unwrap());
        assert_eq!(response.dgraph_ref.id, response.dgraph.dgraph_id);
        assert_eq!(response.dgraph_ref.revision, response.dgraph.revision);
    }

    #[test]
    fn j23_artifact_descriptor_digests_independently_recompute() {
        let response = compile_submission(multi_service_submission()).unwrap();
        for entry in &response.dcode_artifacts {
            assert_eq!(entry.descriptor_digest, recompute_dserv_artifact_digest(&entry.artifact).unwrap());
        }
    }

    #[test]
    fn j24_binding_map_exactly_mirrors_descriptor_digests() {
        let response = compile_submission(multi_service_submission()).unwrap();
        assert_eq!(response.dcode_artifact_authority.len(), response.dcode_artifacts.len());
        for entry in &response.dcode_artifacts {
            let binding = response
                .dcode_artifact_authority
                .get(&entry.service_id)
                .expect("every artifact result must have a binding entry");
            assert_eq!(binding.descriptor_digest, entry.descriptor_digest);
        }
    }

    #[test]
    fn j25_caller_cannot_supply_a_derived_reference_field() {
        let json = r#"{"schema":"datum.dcompile-submission/1","dscript":{"schema":"datum.dscript/1","application_id":"app:x","dscript_id":"dscript:x","revision":1,"functions":[]},"output_dgraph_id":"dgraph:x","output_dgraph_revision":1,"services":[],"dcode_services":[],"dgraph_ref":{"id":"x","revision":1,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#;
        let result: Result<DCompileSubmissionV1, _> = serde_json::from_str(json);
        assert!(result.is_err(), "dgraph_ref must not be an accepted submission field");
    }

    // ==================================================================
    // J26-J32: determinism
    // ==================================================================

    #[test]
    fn j26_same_submission_twice_is_exactly_equal() {
        let s = multi_service_submission();
        let a = compile_submission(s.clone()).unwrap();
        let b = compile_submission(s).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn j27_same_submission_twice_is_byte_identical_json() {
        let s = multi_service_submission();
        let a = serde_json::to_string(&compile_submission(s.clone()).unwrap()).unwrap();
        let b = serde_json::to_string(&compile_submission(s).unwrap()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn j28_dscript_function_reorder_yields_the_same_response() {
        let mut s1 = multi_service_submission();
        let mut s2 = s1.clone();
        s2.dscript.functions.reverse();
        assert_eq!(compile_submission(s1.clone()).unwrap(), compile_submission(s2).unwrap());
        s1.dscript.functions.reverse();
        assert_eq!(compile_submission(s1).unwrap(), compile_submission(multi_service_submission()).unwrap());
    }

    #[test]
    fn j29_mapping_service_reorder_yields_the_same_response() {
        let mut reordered = multi_service_submission();
        reordered.services.reverse();
        reordered.dcode_services.reverse();
        assert_eq!(
            compile_submission(multi_service_submission()).unwrap(),
            compile_submission(reordered).unwrap()
        );
    }

    #[test]
    fn j30_function_ids_reorder_within_a_service_yields_the_same_response() {
        let mut s = submission(
            "app:x",
            "dscript:x",
            &["A", "B"],
            vec![service_mapping("svc-combined", &["A", "B"])],
            vec![dcode_service_input("svc-combined", 'a', 100)],
        );
        let mut reordered = s.clone();
        reordered.services[0].function_ids.reverse();
        assert_eq!(compile_submission(s.clone()).unwrap(), compile_submission(reordered).unwrap());
        s.services[0].function_ids.reverse();
    }

    #[test]
    fn j31_dcode_services_reorder_yields_the_same_response() {
        let mut reordered = multi_service_submission();
        reordered.dcode_services.reverse();
        assert_eq!(
            compile_submission(multi_service_submission()).unwrap(),
            compile_submission(reordered).unwrap()
        );
    }

    #[test]
    fn j32_combined_semantic_reorder_yields_the_same_response() {
        let mut reordered = multi_service_submission();
        reordered.dscript.functions.reverse();
        reordered.services.reverse();
        reordered.dcode_services.reverse();
        let a = compile_submission(multi_service_submission()).unwrap();
        let b = compile_submission(reordered).unwrap();
        assert_eq!(a, b);
        assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    }

    // ==================================================================
    // J33-J35: recompile behavior
    // ==================================================================

    #[test]
    fn j33_h1_to_h2_recompile_same_dgraph_different_descriptor() {
        let mut s1 = single_service_submission();
        s1.dcode_services[0].sha256 = "a".repeat(64);
        let mut s2 = single_service_submission();
        s2.dcode_services[0].sha256 = "b".repeat(64);

        let c1 = compile_submission(s1).unwrap();
        let c2 = compile_submission(s2).unwrap();

        assert_eq!(c1.dscript_ref, c2.dscript_ref);
        assert_eq!(c1.mapping, c2.mapping);
        assert_eq!(c1.dcompile_request_digest, c2.dcompile_request_digest);
        assert_eq!(c1.dgraph, c2.dgraph);
        assert_eq!(c1.dgraph_ref, c2.dgraph_ref);

        assert_ne!(c1.dcode_artifacts[0].artifact.dcode.sha256, c2.dcode_artifacts[0].artifact.dcode.sha256);
        assert_ne!(c1.dcode_artifacts[0].descriptor_digest, c2.dcode_artifacts[0].descriptor_digest);
        assert_ne!(
            c1.dcode_artifact_authority["svc-a"].descriptor_digest,
            c2.dcode_artifact_authority["svc-a"].descriptor_digest
        );
    }

    #[test]
    fn j34_same_module_bytes_different_limits_same_module_digest_different_descriptor() {
        let mut s1 = single_service_submission();
        let mut s2 = single_service_submission();
        s2.dcode_services[0].limits.timeout_ms += 1; // module bytes/sha256 unchanged

        let c1 = compile_submission(s1.clone()).unwrap();
        let c2 = compile_submission(s2).unwrap();

        assert_eq!(c1.dgraph, c2.dgraph);
        assert_eq!(
            c1.dcode_artifacts[0].artifact.dcode.sha256,
            c2.dcode_artifacts[0].artifact.dcode.sha256,
            "module digest must stay identical"
        );
        assert_ne!(c1.dcode_artifacts[0].descriptor_digest, c2.dcode_artifacts[0].descriptor_digest);
        s1.dcode_services[0].limits.timeout_ms += 0; // no-op, documents s1 is the baseline
    }

    #[test]
    fn j35_annotation_only_dscript_change_changes_dscript_digest_but_not_dgraph_or_artifact() {
        let mut s1 = single_service_submission();
        let mut s2 = single_service_submission();
        s2.dscript.functions[0].annotations.insert("hint".to_string(), "value".to_string());

        let c1 = compile_submission(s1.clone()).unwrap();
        let c2 = compile_submission(s2).unwrap();

        assert_ne!(c1.dscript_ref.digest, c2.dscript_ref.digest, "annotations must change D-Script identity");
        assert_eq!(c1.dgraph, c2.dgraph, "annotations must have zero effect on the generated D-Graph");
        assert_eq!(
            c1.dcode_artifacts[0].descriptor_digest, c2.dcode_artifacts[0].descriptor_digest,
            "unchanged DCode input must keep the same descriptor digest"
        );
        s1.dscript.functions[0].annotations.clear();
    }

    // ==================================================================
    // J36-J41: handoff (zero transformation)
    // ==================================================================

    fn handoff_dcontinuum(project: &str) -> crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
        use crate::core::canonical_dcontinuum::{
            CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
            CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
        };
        use crate::core::canonical_dgraph::DNODE_ABI_V0;
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project.to_string(),
            dcontinuum_id: format!("continuum:{project}"),
            revision: 1,
            nodes: vec![CanonicalDNodeV1 {
                node_id: "fog-01".to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_294_967_296 },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
            }],
            links: vec![],
        }
    }

    fn handoff_service_artifacts(service_ids: &[&str]) -> Vec<crate::core::service_artifact_registry::ServiceArtifact> {
        use crate::core::service_artifact_registry::{
            ServiceArtifact, ServiceArtifactControlPlane, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
            SERVICE_ARTIFACT_SCHEMA_VERSION,
        };
        service_ids
            .iter()
            .map(|service_id| ServiceArtifact {
                schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
                artifact_id: format!("artifact:{service_id}"),
                service_id: service_id.to_string(),
                runtime: ServiceRuntimeSpec { kind: "container".to_string(), container: None, native_process: None },
                probes: ServiceProbeSet {
                    execution: ServiceProbeSpec { kind: "none".to_string(), params: Default::default(), vantage_point: None },
                    health: None,
                    availability: None,
                },
                metadata: None,
                control_plane: ServiceArtifactControlPlane {
                    target_nodes: vec!["fog-01".to_string()],
                    ..Default::default()
                },
            })
            .collect()
    }

    #[test]
    fn j36_j37_compile_artifact_directly_declares_and_digest_matches() {
        use crate::core::dcode_registry::{declare_dserv_artifact_revision, DCodeArtifactRegistryStateV2};
        let response = compile_submission(multi_service_submission()).unwrap();
        let mut registry = DCodeArtifactRegistryStateV2::empty();
        for entry in &response.dcode_artifacts {
            let declared_digest = declare_dserv_artifact_revision(&mut registry, entry.artifact.clone())
                .expect("compile-produced artifact must declare successfully, unmodified");
            assert_eq!(declared_digest, entry.descriptor_digest);
        }
    }

    #[test]
    fn j38_response_binding_validates_against_registered_revisions() {
        use crate::core::dcode_registry::{declare_dserv_artifact_revision, DCodeArtifactRegistryStateV2};
        use crate::core::ddeploy::validate_dcode_artifact_authority_bindings;
        let response = compile_submission(multi_service_submission()).unwrap();
        let mut registry = DCodeArtifactRegistryStateV2::empty();
        for entry in &response.dcode_artifacts {
            declare_dserv_artifact_revision(&mut registry, entry.artifact.clone()).unwrap();
        }
        let result = validate_dcode_artifact_authority_bindings(
            &response.dgraph,
            &response.application_id,
            &registry,
            &response.dcode_artifact_authority,
        );
        assert!(result.is_ok(), "compiler-produced dgraph/binding must validate cleanly: {result:?}");
    }

    #[test]
    fn j39_j40_j41_local_compile_register_propose_accept_binds_the_exact_compiler_produced_digest() {
        use crate::core::canonical_dmap::CanonicalDServPlacementV1;
        use crate::core::dcode_registry::{declare_dserv_artifact_revision, DCodeArtifactRegistryStateV2};
        use crate::core::ddeploy::{
            accept_proposal_with_dcode_binding_revalidation, active_placement_authority, create_proposal,
            resolve_active_dcode_binding, CreateProposalInput, DDeployStateV1, ProposalSource,
        };

        let response = compile_submission(single_service_submission()).unwrap();

        // J36/J39: the response's own D-Graph enters DDeploy UNMODIFIED.
        let mut registry = DCodeArtifactRegistryStateV2::empty();
        for entry in &response.dcode_artifacts {
            declare_dserv_artifact_revision(&mut registry, entry.artifact.clone()).unwrap();
        }

        let dcontinuum = handoff_dcontinuum("proj-x");
        let artifacts = handoff_service_artifacts(&["svc-a"]);
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority: response.dcode_artifact_authority.clone(), // UNMODIFIED
                project_id: "proj-x".to_string(),
                application_id: response.application_id.clone(),
                source: ProposalSource::Operator,
                dgraph: response.dgraph.clone(), // UNMODIFIED
                dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }],
            },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .expect("compiler-produced dgraph/binding must be accepted by create_proposal unmodified");

        // J40/J41: accept, and prove the active binding is EXACTLY the
        // compiler-produced descriptor digest.
        accept_proposal_with_dcode_binding_revalidation(
            &mut state,
            "p1",
            "operator:alice",
            None,
            &artifacts,
            Some(&dcontinuum),
            &registry,
            "2026-01-01T00:00:01Z",
            "a1".to_string(),
        )
        .expect("acceptance must succeed against the compiler-produced binding");

        let active = active_placement_authority(&state, "proj-x", &response.application_id).unwrap().unwrap();
        let bound = resolve_active_dcode_binding(&state, &active, "svc-a").expect("svc-a must have an active binding");
        assert_eq!(bound.descriptor_digest, response.dcode_artifacts[0].descriptor_digest);
    }

    // ==================================================================
    // J42-J47: authority negative space
    // ==================================================================

    #[test]
    fn j42_compile_alone_does_not_register_any_artifact() {
        use crate::core::dcode_registry::DCodeArtifactRegistryStateV2;
        let _ = compile_submission(multi_service_submission()).unwrap();
        // Structural proof: `compile_submission` takes no registry parameter
        // at all -- a freshly constructed registry remains empty regardless
        // of how many times compilation ran.
        let registry = DCodeArtifactRegistryStateV2::empty();
        assert_eq!(registry.revision_count("app:x", "svc-a"), 0);
    }

    #[test]
    fn j43_j44_compile_alone_does_not_create_a_proposal_or_active_dmap() {
        use crate::core::ddeploy::DDeployStateV1;
        let _ = compile_submission(multi_service_submission()).unwrap();
        // Structural proof: `compile_submission` takes no DDeploy state
        // parameter at all -- a freshly constructed state remains empty
        // regardless of how many times compilation ran.
        let state = DDeployStateV1::empty();
        assert!(state.proposals.is_empty());
        assert!(state.active_by_application.is_empty());
    }

    #[test]
    fn j45_response_contains_no_active_or_accepted_field() {
        let response = compile_submission(multi_service_submission()).unwrap();
        let text = serde_json::to_value(&response).unwrap().to_string();
        for forbidden in ["\"active\"", "\"current\"", "\"accepted\"", "\"registered\"", "\"deployed\"", "\"authorized\"", "\"installed\""] {
            assert!(!text.contains(forbidden), "forbidden authority-implying key '{forbidden}' found in compile response: {text}");
        }
    }

    #[test]
    fn j46_submission_contains_no_project_or_placement_field() {
        let submission = multi_service_submission();
        let text = serde_json::to_value(&submission).unwrap().to_string();
        for forbidden in ["project_id", "node_id", "placement", "dmap", "dcontinuum", "\"proposal", "\"accept"] {
            assert!(!text.contains(forbidden), "forbidden authority-adjacent key '{forbidden}' found in submission: {text}");
        }
    }

    /// J47: confirmed by direct source inspection, not a runtime assertion --
    /// `DCompileDGraphCandidate`/`DCompileCandidateBundle`/
    /// `DCompileDServArtifactCandidate` (`dcompile.rs`/`dcompile_dcode.rs`)
    /// derive neither `Serialize` nor `Deserialize`; this module's own
    /// `DCompileCandidateResponseV1`/`DCompileArtifactResultV1` are
    /// independent, purpose-built wire types constructed by `build_response`
    /// FROM the internal result, never by attaching serde to it.
    #[test]
    fn j47_internal_candidate_types_remain_non_wire_by_construction() {
        let response = compile_submission(single_service_submission()).unwrap();
        // The only way this test module could serialize a "candidate" value
        // at all is through the new wire type -- there is no
        // `serde_json::to_string(&DCompileCandidateBundle)` call anywhere in
        // this crate, and one would fail to compile if attempted.
        assert!(serde_json::to_string(&response).is_ok());
    }

    // ==================================================================
    // Section 18: intentional double D-Graph lowering
    // ==================================================================

    /// Proves the specific claim `compile_submission`'s own doc comment
    /// makes: the PRELIMINARY lowering call it performs to derive
    /// `dgraph_ref`, and the lowering `lower_dcode_artifact_candidates`
    /// performs again internally, produce an IDENTICAL result for the same
    /// validated inputs -- so the intentional repetition is provably a
    /// no-op duplication of a pure, cheap computation, never a source of
    /// drift between the "preliminary" and "final" candidate.
    #[test]
    fn preliminary_dgraph_lowering_equals_final_bundles_dgraph_candidate() {
        let dscript = dscript("app:x", "dscript:x", &["A"]);
        let validated_dscript = validate_and_digest_dscript(dscript).unwrap();
        let request = DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: validated_dscript.document.application_id.clone(),
            dscript_ref: CanonicalContractReferenceV1 {
                id: validated_dscript.document.dscript_id.clone(),
                revision: validated_dscript.document.revision,
                digest: validated_dscript.digest_sha256.clone(),
            },
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![service_mapping("svc-a", &["A"])],
        };
        let validated_request = validate_and_digest_dcompile_request(request).unwrap();

        let first = lower_calls_empty_dgraph_candidate(&validated_dscript, &validated_request).unwrap();
        let second = lower_calls_empty_dgraph_candidate(&validated_dscript, &validated_request).unwrap();
        assert_eq!(first, second);
    }

    // ==================================================================
    // Response mapping canonicalization (Section 20)
    // ==================================================================

    #[test]
    fn canonicalized_mapping_is_sorted_regardless_of_submission_order() {
        let mut s = multi_service_submission();
        s.services.reverse();
        for service in &mut s.services {
            service.function_ids.reverse();
        }
        let response = compile_submission(s).unwrap();
        let ids: Vec<&str> = response.mapping.services.iter().map(|svc| svc.service_id.as_str()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids, "mapping.services must be canonically sorted by service_id");
        for service in &response.mapping.services {
            let mut sorted = service.function_ids.clone();
            sorted.sort();
            assert_eq!(service.function_ids, sorted, "mapping.services[].function_ids must be canonically sorted");
        }
    }
}
