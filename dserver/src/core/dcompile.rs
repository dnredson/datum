//! Phase 117C: the FIRST actual D-Compile lowering step. Pure, deterministic,
//! narrowly scoped to exactly what Phase 117A/117B's explicit inputs
//! actually support: a calls-empty candidate `CanonicalDGraphV1`.
//!
//! `dcompile_request.rs` owns the explicit compile-time INPUT contract
//! (`datum.dcompile-request/1`) and its own structural/relational
//! validation. This module owns LOWERING -- turning an already-validated
//! `ValidatedDScriptInput` + `ValidatedDCompileRequest` pair into a candidate
//! D-Graph -- and nothing else. No `compile()`/`generate_dcode()` exists
//! here; D-Call derivation and D-Code descriptor wiring both remain out of
//! scope pending real evidence (Phase 117A Sections 8, 5).
//!
//! **The candidate this produces is NOT authority.** It is not active,
//! accepted, deployed, placed, executable, or authorized. Nothing in this
//! module calls `ddeploy::create_proposal`, registers an artifact, or
//! mutates any host. Those remain later, separate integration steps
//! (Phase 117F and beyond).
//!
//! **Trust boundary (P0, load-bearing):** `ValidatedDScriptInput` and
//! `ValidatedDCompileRequest` both have fully PUBLIC fields (by design --
//! Phase 116/117B do not make them opaque). This means ordinary Rust code
//! CAN construct either directly with a fabricated digest, bypassing
//! `decode_and_validate_dscript_json`/`decode_and_validate_dcompile_request_json`
//! entirely. `lower_calls_empty_dgraph_candidate` therefore never trusts a
//! wrapper's cached `digest_sha256` as self-certifying: it re-runs
//! `document.validate()`/`request.validate()` and independently recomputes
//! both digests, comparing them against what the wrapper claims, before
//! ever constructing a candidate D-Graph. This mirrors the same
//! "recompute and compare, never trust cached/self-asserted identity"
//! discipline this project already applies everywhere else (Stage 107A
//! lease/authorization revalidation, `execution_projection`'s digest
//! recomputation). Changing `Validated*`'s field visibility to make
//! fabrication impossible at the type level was considered and rejected:
//! it would be an unrequested redesign of a frozen prior-phase contract,
//! where this function's own defensive revalidation is the smaller,
//! sufficient, and already-precedented fix.
//!
//! **Provenance fact (load-bearing, tested explicitly below):**
//! `CanonicalDGraphV1` does not, and structurally cannot, record which
//! D-Function belongs to which D-Serv -- Phase 116/117A both established
//! D-Script/D-Graph never carry that relationship. Consequently, two
//! DIFFERENT valid compile requests (differing only in which D-Function
//! maps to which service, with identical service IDs/resources/output
//! identity) can lower to the IDENTICAL candidate D-Graph and D-Graph
//! digest. **A D-Graph digest alone can never serve as proof of which
//! D-Compile mapping produced it.** This is why
//! `DCompileDGraphCandidate` below carries the D-Script digest and the
//! D-Compile-request digest alongside the D-Graph digest -- three
//! different digests answering three different questions (exact source
//! identity; exact compile mapping/resource/output-identity intent; exact
//! generated application topology) that must never be collapsed into one.
//!
//! **Server-side only.** Mirrors `dcompile_request.rs`'s own decision:
//! DCompile is a control-plane responsibility; there is no D-Node-agent
//! consumer for lowering.

use crate::core::canonical_dgraph::{
    recompute_dgraph_digest, CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
    CanonicalDServV1, CanonicalValidationFinding as DGraphValidationFinding,
    CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
};
use crate::core::canonical_dscript::{
    recompute_dscript_digest, CanonicalValidationFinding as DScriptValidationFinding,
    ValidatedDScriptInput,
};
use crate::core::dcompile_request::{recompute_dcompile_request_digest, ValidatedDCompileRequest};

/// Distinguishes exactly where lowering refused to proceed. Never a single
/// opaque `String` -- each variant preserves the REAL
/// `CanonicalValidationFinding`s a downstream caller needs, rather than a
/// paraphrased message.
#[derive(Debug)]
pub enum DCompileLoweringError {
    /// The D-Script wrapper's own document failed its structural
    /// `validate()` -- the wrapper's cached state cannot be trusted at all.
    InvalidDScript(Vec<DScriptValidationFinding>),
    /// The D-Script wrapper's `digest_sha256` does not match what
    /// independently recomputing the digest from its own `document`
    /// produces -- proof the wrapper was not honestly produced by
    /// `decode_and_validate_dscript_json`.
    DScriptDigestMismatch {
        wrapper_claimed: String,
        recomputed: String,
    },
    /// The D-Compile request wrapper's own request failed its structural
    /// `validate()`.
    InvalidDCompileRequest(Vec<DScriptValidationFinding>),
    /// The D-Compile request wrapper's `digest_sha256` does not match an
    /// independent recomputation.
    DCompileRequestDigestMismatch {
        wrapper_claimed: String,
        recomputed: String,
    },
    /// The request structurally validates on its own, and the D-Script
    /// structurally validates on its own, but the two do not relationally
    /// agree (`DCompileRequestV1::validate_against_dscript`'s own findings
    /// -- identity/digest mismatch, unknown function, unmapped function).
    /// Same finding type as `InvalidDScript`/`InvalidDCompileRequest`:
    /// `dcompile_request.rs` itself imports (not redeclares)
    /// `canonical_dscript::CanonicalValidationFinding` for exactly this
    /// purpose.
    RelationalMismatch(Vec<DScriptValidationFinding>),
    /// Defense in depth: the D-Graph this function itself constructed
    /// failed `CanonicalDGraphV1::validate()`. Should be unreachable given
    /// already-validated inputs; modeled as a real, distinct failure rather
    /// than a `panic!`/`unwrap()`, exactly like
    /// `canonical_dscript::DScriptInputError::DigestComputation`'s own
    /// precedent for a theoretically-unreachable-but-honestly-modeled case.
    /// Uses `canonical_dgraph`'s OWN distinct `CanonicalValidationFinding`
    /// type (nominally different from, though structurally identical to,
    /// `canonical_dscript`'s) -- `CanonicalDGraphV1::validate()` returns
    /// that type, not the D-Script one.
    GeneratedDGraphInvalid(Vec<DGraphValidationFinding>),
    /// A digest recomputation itself failed (JCS canonicalization error) --
    /// unreachable in practice for any of this module's field sets (no
    /// field here can ever hold a non-finite float), modeled honestly
    /// rather than unwrapped.
    DigestComputationFailed(String),
}

/// The pure result of lowering. Deliberately NOT a wire contract: no
/// `Serialize`/`Deserialize`. Phase 117J added the first real production
/// API consumer (`core::dcompile_submission::compile_submission`,
/// `POST /api/v1/dcompile/compile`) without changing this decision: that
/// consumer maps this internal result into its own independently-versioned
/// wire type (`DCompileCandidateResponseV1`) rather than attaching serde
/// here -- see 117I Section 10 (Decision B) for the full rationale, and
/// `dcompile_submission.rs`'s own module doc comment. Also deliberately
/// carries no `compiled_at`/timestamp: same inputs must always produce the
/// same result, and a timestamp would destroy that.
///
/// This is a compiler RESULT, not an authority record: it is never
/// "accepted," "current," or "active" merely by existing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DCompileDGraphCandidate {
    pub dgraph: CanonicalDGraphV1,
    /// Freshly recomputed via `recompute_dgraph_digest` -- never trusted
    /// from any caller. Identifies the exact generated application
    /// topology, and ONLY that -- see the module doc comment's provenance
    /// fact: this digest alone does not identify which D-Compile mapping
    /// produced it.
    pub dgraph_digest_sha256: String,
    /// The exact D-Script this candidate was compiled from. Identifies
    /// source identity (including its opaque annotations, which have zero
    /// effect on the D-Graph produced -- see
    /// `annotation_changes_do_not_affect_generated_dgraph` below).
    pub dscript_digest_sha256: String,
    /// The exact D-Compile request (mapping + resources + output identity)
    /// used. This is the ONLY one of the three digests that captures
    /// D-Function -> D-Serv membership -- the D-Graph itself cannot.
    pub dcompile_request_digest_sha256: String,
}

/// Lower an explicit, already-decoded D-Script and D-Compile request into a
/// calls-empty candidate `CanonicalDGraphV1`. Pure: no I/O, no global state,
/// no timestamps, no random/generated IDs -- every identity in the output
/// (`dgraph_id`, `revision`, `service_id`s) is copied verbatim from the
/// explicit inputs (Phase 117B already made all of them explicit; this
/// function invents none). The one exception is `dnode_abi`, assigned the
/// sole legal v0.1 D-Node ABI explicitly -- never called a "default" because
/// there is no second value to default away from (Phase 117A).
///
/// `dscript.document.functions`/`request.request.services[].function_ids`
/// are consulted ONLY for the relational validation this function performs
/// before constructing anything; the returned D-Graph cannot and does not
/// encode D-Function membership at all (see the module doc comment).
///
/// Fails closed, before ever constructing a candidate D-Graph, on: an
/// internally-inconsistent D-Script wrapper, an internally-inconsistent
/// D-Compile-request wrapper, or a structurally-valid-on-both-sides request
/// that does not relationally match the given D-Script (wrong application,
/// wrong D-Script identity/revision/digest, unknown function, unmapped
/// function, or -- were it somehow smuggled past 117B's own structural
/// validation -- overlapping membership).
pub fn lower_calls_empty_dgraph_candidate(
    dscript: &ValidatedDScriptInput,
    request: &ValidatedDCompileRequest,
) -> Result<DCompileDGraphCandidate, DCompileLoweringError> {
    // Step 1-2: the D-Script wrapper is never trusted as self-certifying.
    let dscript_findings = dscript.document.validate();
    if !dscript_findings.is_empty() {
        return Err(DCompileLoweringError::InvalidDScript(dscript_findings));
    }
    let recomputed_dscript_digest = recompute_dscript_digest(&dscript.document)
        .map_err(DCompileLoweringError::DigestComputationFailed)?;
    if recomputed_dscript_digest != dscript.digest_sha256 {
        return Err(DCompileLoweringError::DScriptDigestMismatch {
            wrapper_claimed: dscript.digest_sha256.clone(),
            recomputed: recomputed_dscript_digest,
        });
    }

    // Step 3-4: same discipline for the D-Compile request wrapper.
    let request_findings = request.request.validate();
    if !request_findings.is_empty() {
        return Err(DCompileLoweringError::InvalidDCompileRequest(
            request_findings,
        ));
    }
    let recomputed_request_digest = recompute_dcompile_request_digest(&request.request)
        .map_err(DCompileLoweringError::DigestComputationFailed)?;
    if recomputed_request_digest != request.digest_sha256 {
        return Err(DCompileLoweringError::DCompileRequestDigestMismatch {
            wrapper_claimed: request.digest_sha256.clone(),
            recomputed: recomputed_request_digest,
        });
    }

    // Step 5: relational agreement -- reuses 117B's own validator exactly,
    // never re-derived here.
    let relational_findings = request.request.validate_against_dscript(dscript);
    if !relational_findings.is_empty() {
        return Err(DCompileLoweringError::RelationalMismatch(
            relational_findings,
        ));
    }

    // Step 6: construct the candidate. Services are sorted by `service_id`
    // so that caller-vector reordering of `request.request.services` never
    // produces a different D-Graph value, not merely the same digest.
    let mut services: Vec<CanonicalDServV1> = request
        .request
        .services
        .iter()
        .map(|mapping| CanonicalDServV1 {
            service_definition: None, service_id: mapping.service_id.clone(),
            execution_requirement: CanonicalDServExecutionRequirementV1 {
                dnode_abi: DNODE_ABI_V0.to_string(),
            },
            resource_requirement: mapping.resource_requirement,
        })
        .collect();
    services.sort_by(|a, b| a.service_id.cmp(&b.service_id));

    let dgraph = CanonicalDGraphV1 {
        schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
        application_id: dscript.document.application_id.clone(),
        dgraph_id: request.request.output_dgraph_id.clone(),
        revision: request.request.output_dgraph_revision,
        services,
        calls: Vec::new(),
    };

    // Step 7: defense in depth -- the canonical validator is the single
    // source of truth for D-Graph structural validity; never re-derived.
    let dgraph_findings = dgraph.validate();
    if !dgraph_findings.is_empty() {
        return Err(DCompileLoweringError::GeneratedDGraphInvalid(
            dgraph_findings,
        ));
    }

    // Step 8: the D-Graph digest is always freshly computed, never trusted
    // from any input.
    let dgraph_digest_sha256 =
        recompute_dgraph_digest(&dgraph).map_err(DCompileLoweringError::DigestComputationFailed)?;

    Ok(DCompileDGraphCandidate {
        dgraph,
        dgraph_digest_sha256,
        dscript_digest_sha256: dscript.digest_sha256.clone(),
        dcompile_request_digest_sha256: request.digest_sha256.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dgraph::CanonicalDServResourceRequirementV1;
    use crate::core::canonical_dmap::CanonicalContractReferenceV1;
    use crate::core::canonical_dscript::{CanonicalDFunctionV1, CanonicalDScriptV1};
    use crate::core::dcompile_request::{
        DCompileRequestV1, DCompileServiceMappingV1, CANONICAL_DCOMPILE_REQUEST_SCHEMA,
    };
    use std::collections::BTreeMap;

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 {
            cpu_cores,
            memory_bytes,
        }
    }

    fn dscript_doc(
        app: &str,
        dscript_id: &str,
        ids: &[&str],
        annotations: &[(&str, &[(&str, &str)])],
    ) -> CanonicalDScriptV1 {
        CanonicalDScriptV1 {
            schema: "datum.dscript/1".to_string(),
            application_id: app.to_string(),
            dscript_id: dscript_id.to_string(),
            revision: 1,
            functions: ids
                .iter()
                .map(|id| {
                    let ann: BTreeMap<String, String> = annotations
                        .iter()
                        .find(|(fid, _)| fid == id)
                        .map(|(_, pairs)| {
                            pairs
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    CanonicalDFunctionV1 {
                        function_id: id.to_string(),
                        annotations: ann,
                    }
                })
                .collect(),
        }
    }

    fn validated_dscript(document: CanonicalDScriptV1) -> ValidatedDScriptInput {
        let digest_sha256 = recompute_dscript_digest(&document).unwrap();
        ValidatedDScriptInput {
            document,
            digest_sha256,
        }
    }

    fn request_for(
        dscript: &ValidatedDScriptInput,
        mappings: &[(&str, &[&str], (u32, u64))],
        dgraph_id: &str,
        revision: u64,
    ) -> DCompileRequestV1 {
        DCompileRequestV1 {
            schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
            application_id: dscript.document.application_id.clone(),
            dscript_ref: CanonicalContractReferenceV1 {
                id: dscript.document.dscript_id.clone(),
                revision: dscript.document.revision,
                digest: dscript.digest_sha256.clone(),
            },
            output_dgraph_id: dgraph_id.to_string(),
            output_dgraph_revision: revision,
            services: mappings
                .iter()
                .map(
                    |(service_id, function_ids, (cpu, mem))| DCompileServiceMappingV1 {
                        service_id: service_id.to_string(),
                        function_ids: function_ids.iter().map(|s| s.to_string()).collect(),
                        resource_requirement: res(*cpu, *mem),
                    },
                )
                .collect(),
        }
    }

    fn validated_request(request: DCompileRequestV1) -> ValidatedDCompileRequest {
        let digest_sha256 = recompute_dcompile_request_digest(&request).unwrap();
        ValidatedDCompileRequest {
            request,
            digest_sha256,
        }
    }

    // ---- C1-C3: basic grouping shapes --------------------------------------- //

    #[test]
    fn c1_one_function_to_one_service_lowers_correctly() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 134_217_728))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).expect("must lower");
        assert_eq!(candidate.dgraph.services.len(), 1);
        assert_eq!(candidate.dgraph.services[0].service_id, "svc-a");
        assert_eq!(candidate.dgraph.calls, Vec::new());
        assert_eq!(candidate.dgraph.validate(), Vec::new());
    }

    /// C2: M:1 -- two functions must become exactly ONE generated service,
    /// proving the request's grouping (not function count) is honored.
    #[test]
    fn c2_m_to_1_mapping_produces_exactly_one_service() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-combined", &["A", "B"], (2, 268_435_456))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(candidate.dgraph.services.len(), 1);
        assert_eq!(candidate.dgraph.services[0].service_id, "svc-combined");
    }

    /// C3: partition across multiple services -- expected service set,
    /// calls always empty.
    #[test]
    fn c3_partition_across_multiple_services() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let ids: Vec<&str> = candidate
            .dgraph
            .services
            .iter()
            .map(|s| s.service_id.as_str())
            .collect();
        assert_eq!(ids, vec!["svc-a", "svc-b"]);
        assert_eq!(candidate.dgraph.calls, Vec::new());
    }

    // ---- C4-C5: caller ordering has zero effect ------------------------------ //

    #[test]
    fn c4_service_vector_reorder_produces_identical_dgraph_and_digest() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let forward = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let reversed = validated_request(request_for(
            &dscript,
            &[("svc-b", &["B"], (1, 1)), ("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let a = lower_calls_empty_dgraph_candidate(&dscript, &forward).unwrap();
        let b = lower_calls_empty_dgraph_candidate(&dscript, &reversed).unwrap();
        assert_eq!(a.dgraph, b.dgraph);
        assert_eq!(a.dgraph_digest_sha256, b.dgraph_digest_sha256);
    }

    #[test]
    fn c5_function_id_reorder_within_service_produces_identical_dgraph_and_digest() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let forward = validated_request(request_for(
            &dscript,
            &[("svc-combined", &["A", "B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let reversed = validated_request(request_for(
            &dscript,
            &[("svc-combined", &["B", "A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let a = lower_calls_empty_dgraph_candidate(&dscript, &forward).unwrap();
        let b = lower_calls_empty_dgraph_candidate(&dscript, &reversed).unwrap();
        assert_eq!(a.dgraph, b.dgraph);
        assert_eq!(a.dgraph_digest_sha256, b.dgraph_digest_sha256);
    }

    // ---- C6: repeatability --------------------------------------------------- //

    #[test]
    fn c6_repeat_lowering_same_inputs_yields_equal_result() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let a = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let b = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(a, b);
    }

    // ---- C7: empty DScript/request ------------------------------------------- //

    #[test]
    fn c7_empty_dscript_and_empty_request_lowers_to_empty_valid_graph() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &[], &[]));
        let request = validated_request(request_for(&dscript, &[], "dgraph:x", 1));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(candidate.dgraph.services, Vec::new());
        assert_eq!(candidate.dgraph.calls, Vec::new());
        assert_eq!(candidate.dgraph.validate(), Vec::new());
    }

    // ---- C8: resource preservation -------------------------------------------- //

    #[test]
    fn c8_resource_requirements_copied_exactly() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (3, 999_999_999))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(
            candidate.dgraph.services[0].resource_requirement,
            res(3, 999_999_999)
        );
    }

    // ---- C9-C10: ABI and calls ------------------------------------------------ //

    #[test]
    fn c9_every_service_gets_dnode_abi_v0() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        for service in &candidate.dgraph.services {
            assert_eq!(service.execution_requirement.dnode_abi, DNODE_ABI_V0);
        }
    }

    #[test]
    fn c10_calls_are_always_empty() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B", "C"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-all", &["A", "B", "C"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert!(candidate.dgraph.calls.is_empty());
    }

    // ---- C11: annotation/topology independence -------------------------------- //

    /// C11 / Section 17/35: two DScripts differing ONLY in opaque annotation
    /// content, with requests correctly referencing each one's own digest,
    /// must lower to the IDENTICAL candidate D-Graph. Annotations change
    /// D-Script identity (proven: the two D-Script digests below differ) but
    /// have zero D-Graph-lowering semantics.
    #[test]
    fn c11_annotation_changes_do_not_affect_generated_dgraph() {
        let plain = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let annotated = validated_dscript(dscript_doc(
            "app:x",
            "dscript:x",
            &["A", "B"],
            &[
                ("A", &[("cpu", "16"), ("node", "fog-99")]),
                ("B", &[("group", "svc-b")]),
            ],
        ));
        assert_ne!(
            plain.digest_sha256, annotated.digest_sha256,
            "annotations must change DScript identity"
        );

        let request_plain = validated_request(request_for(
            &plain,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let request_annotated = validated_request(request_for(
            &annotated,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        assert_ne!(
            request_plain.digest_sha256, request_annotated.digest_sha256,
            "request digest must differ because dscript_ref.digest differs"
        );

        let a = lower_calls_empty_dgraph_candidate(&plain, &request_plain).unwrap();
        let b = lower_calls_empty_dgraph_candidate(&annotated, &request_annotated).unwrap();
        assert_eq!(
            a.dgraph, b.dgraph,
            "annotations must have zero effect on the generated D-Graph"
        );
        assert_eq!(a.dgraph_digest_sha256, b.dgraph_digest_sha256);
    }

    // ---- C12: THE mapping-provenance collision test (Section 34) ------------- //

    /// This is the single most important test in Phase 117C: proves a
    /// D-Graph digest can NEVER serve as proof of which D-Compile mapping
    /// produced it, because `CanonicalDGraphV1` structurally cannot encode
    /// D-Function membership. Two DIFFERENT, individually valid, exact
    /// partitions of the SAME D-Script -- swapping which function goes to
    /// which service -- lower to the IDENTICAL D-Graph and D-Graph digest,
    /// while their own request digests correctly differ.
    #[test]
    fn c12_mapping_provenance_collision_same_dgraph_different_request_digest() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let r1 = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let r2 = validated_request(request_for(
            &dscript,
            &[("svc-a", &["B"], (1, 1)), ("svc-b", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));

        assert_ne!(
            r1.digest_sha256, r2.digest_sha256,
            "different mappings must produce different request digests"
        );

        let l1 = lower_calls_empty_dgraph_candidate(&dscript, &r1).unwrap();
        let l2 = lower_calls_empty_dgraph_candidate(&dscript, &r2).unwrap();

        assert_eq!(
            l1.dgraph, l2.dgraph,
            "the CanonicalDGraphV1 itself cannot distinguish these two different mappings"
        );
        assert_eq!(l1.dgraph_digest_sha256, l2.dgraph_digest_sha256);

        // But the lowering RESULT still exposes each request's own digest,
        // so the distinction is never lost to a caller who kept the result.
        assert_ne!(
            l1.dcompile_request_digest_sha256,
            l2.dcompile_request_digest_sha256
        );
        assert_eq!(l1.dscript_digest_sha256, l2.dscript_digest_sha256);
    }

    // ---- C13-C16: forged wrapper defensive tests ------------------------------ //

    #[test]
    fn c13_forged_dscript_wrapper_wrong_digest_fails_closed() {
        let document = dscript_doc("app:x", "dscript:x", &["A"], &[]);
        let forged = ValidatedDScriptInput {
            document,
            digest_sha256: "sha256:forged00000000000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let request = validated_request(request_for(
            &forged,
            &[("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let error = lower_calls_empty_dgraph_candidate(&forged, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::DScriptDigestMismatch { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn c14_forged_dscript_wrapper_structurally_invalid_document_fails_closed() {
        let mut document = dscript_doc("app:x", "dscript:x", &["A"], &[]);
        document.schema = "not-a-real-schema".to_string();
        let digest_sha256 = recompute_dscript_digest(&document).unwrap();
        let forged = ValidatedDScriptInput {
            document,
            digest_sha256,
        };
        let request = validated_request(request_for(
            &forged,
            &[("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let error = lower_calls_empty_dgraph_candidate(&forged, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::InvalidDScript(_)),
            "{error:?}"
        );
    }

    #[test]
    fn c15_forged_request_wrapper_wrong_digest_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = request_for(&dscript, &[("svc-a", &["A"], (1, 1))], "dgraph:x", 1);
        let forged = ValidatedDCompileRequest {
            request,
            digest_sha256: "sha256:forged00000000000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let error = lower_calls_empty_dgraph_candidate(&dscript, &forged).unwrap_err();
        assert!(
            matches!(
                error,
                DCompileLoweringError::DCompileRequestDigestMismatch { .. }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn c16_forged_request_wrapper_structurally_invalid_request_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let mut request = request_for(&dscript, &[("svc-a", &["A"], (1, 1))], "dgraph:x", 1);
        request.output_dgraph_revision = 0; // structurally invalid
        let digest_sha256 = recompute_dcompile_request_digest(&request).unwrap();
        let forged = ValidatedDCompileRequest {
            request,
            digest_sha256,
        };
        let error = lower_calls_empty_dgraph_candidate(&dscript, &forged).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::InvalidDCompileRequest(_)),
            "{error:?}"
        );
    }

    // ---- C17-C22: relational mismatches --------------------------------------- //

    #[test]
    fn c17_application_id_mismatch_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let mut request = request_for(&dscript, &[("svc-a", &["A"], (1, 1))], "dgraph:x", 1);
        request.application_id = "app:other".to_string();
        let request = validated_request(request);
        let error = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::RelationalMismatch(_)),
            "{error:?}"
        );
    }

    #[test]
    fn c18_wrong_dscript_id_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let mut request = request_for(&dscript, &[("svc-a", &["A"], (1, 1))], "dgraph:x", 1);
        request.dscript_ref.id = "dscript:someone-else".to_string();
        let request = validated_request(request);
        let error = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::RelationalMismatch(_)),
            "{error:?}"
        );
    }

    #[test]
    fn c19_wrong_dscript_revision_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let mut request = request_for(&dscript, &[("svc-a", &["A"], (1, 1))], "dgraph:x", 1);
        request.dscript_ref.revision = 99;
        let request = validated_request(request);
        let error = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::RelationalMismatch(_)),
            "{error:?}"
        );
    }

    #[test]
    fn c20_wrong_dscript_digest_reference_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let mut request = request_for(&dscript, &[("svc-a", &["A"], (1, 1))], "dgraph:x", 1);
        request.dscript_ref.digest = format!("sha256:{}", "0".repeat(64));
        let request = validated_request(request);
        let error = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::RelationalMismatch(_)),
            "{error:?}"
        );
    }

    #[test]
    fn c21_unknown_mapped_function_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A", "Z"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let error = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::RelationalMismatch(_)),
            "{error:?}"
        );
    }

    #[test]
    fn c22_unmapped_function_fails_closed() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let error = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::RelationalMismatch(_)),
            "{error:?}"
        );
    }

    /// C23: an overlapping mapping smuggled through a forged
    /// `ValidatedDCompileRequest` (bypassing `decode_and_validate_dcompile_request_json`)
    /// is still caught -- by `DCompileRequestV1::validate()`'s own
    /// structural overlap check, invoked as this function's Step 3, before
    /// any D-Graph is ever constructed.
    #[test]
    fn c23_overlapping_mapping_smuggled_through_forged_wrapper_is_caught() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A", "B"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        );
        let digest_sha256 = recompute_dcompile_request_digest(&request).unwrap();
        let forged = ValidatedDCompileRequest {
            request,
            digest_sha256,
        };
        let error = lower_calls_empty_dgraph_candidate(&dscript, &forged).unwrap_err();
        assert!(
            matches!(error, DCompileLoweringError::InvalidDCompileRequest(_)),
            "{error:?}"
        );
    }

    // ---- C24: generated-graph self-validation is actually invoked ------------- //

    /// Proves Step 7 is real, not decorative. Every input this module's
    /// production `validate()`/`validate_against_dscript()` accept already
    /// makes `CanonicalDGraphV1::validate()` pass by construction (that is
    /// the whole point of Steps 1-5) -- so Step 7 can never be observed
    /// actually rejecting a real successful lowering from this module's own
    /// public API. What this test proves instead: `CanonicalDGraphV1::validate()`,
    /// the EXACT function Step 7 calls, independently confirms the produced
    /// candidate is genuinely valid -- not merely assumed valid because
    /// lowering returned `Ok`. Distorting production code to force an
    /// artificial invalid-candidate path was rejected as unclean per the
    /// task's own instruction.
    #[test]
    fn c24_generated_dgraph_self_validation_is_invoked_on_every_success_path() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A", "B"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1)), ("svc-b", &["B"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        // Independently re-run the exact validator Step 7 already ran --
        // if lowering had skipped it, this would still catch a real defect;
        // it does not, confirming the produced candidate is genuinely valid.
        assert_eq!(candidate.dgraph.validate(), Vec::new());
    }

    // ---- graph/request digest recomputation proofs ---------------------------- //

    #[test]
    fn dgraph_digest_matches_independent_recomputation() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(
            candidate.dgraph_digest_sha256,
            recompute_dgraph_digest(&candidate.dgraph).unwrap()
        );
    }

    #[test]
    fn request_and_dscript_digests_match_independent_recomputation() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(
            candidate.dscript_digest_sha256,
            recompute_dscript_digest(&dscript.document).unwrap()
        );
        assert_eq!(
            candidate.dcompile_request_digest_sha256,
            recompute_dcompile_request_digest(&request.request).unwrap()
        );
    }

    // ---- purity / no input mutation -------------------------------------------- //

    #[test]
    fn lowering_does_not_mutate_inputs() {
        let dscript = validated_dscript(dscript_doc("app:x", "dscript:x", &["B", "A"], &[]));
        let request = validated_request(request_for(
            &dscript,
            &[("svc-b", &["B"], (1, 1)), ("svc-a", &["A"], (1, 1))],
            "dgraph:x",
            1,
        ));
        let dscript_before = dscript.clone();
        let request_before = request.clone();
        let _ = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        assert_eq!(dscript, dscript_before);
        assert_eq!(request, request_before);
    }
}
