//! Production D-Compile HTTP surface (Phase 117J). Exposes exactly ONE
//! endpoint, `POST /api/v1/dcompile/compile`, the first real production
//! caller of the Phase 117 compiler pipeline (117B-E). See
//! `documentation/reference/phase117i-production-dcompile-api-handoff-audit.md`
//! for the full design and
//! `documentation/reference/phase117j-production-dcompile-api.md` for what
//! this phase actually implemented.
//!
//! This handler is PURE and STATELESS: it touches no control-state DB, no
//! D-Code registry, no D-Deploy state, no D-Node registry, no filesystem, no
//! network, and acquires none of `DDEPLOY_TRANSACTION`/`DCODE_TRANSACTION`/
//! `DNODE_TRANSACTION`. `core::dcompile_submission::compile_submission` does
//! all real work; this file only deserializes the request, calls it, and
//! maps the result onto HTTP.
//!
//! A successful response is CANDIDATE DATA ONLY -- it never registers a
//! D-Code revision, creates or accepts a D-Deploy proposal, selects
//! placement, or installs module bytes. Those remain separate, explicit,
//! later transitions the caller performs itself against the existing
//! `/api/v1/dcode/artifacts` and `/api/v1/ddeploy/proposals` endpoints.

use axum::{extract::Json, http::StatusCode, Json as JsonResponse};
use serde_json::{json, Value};

use crate::core::canonical_dgraph::CanonicalValidationFinding as DGraphValidationFinding;
use crate::core::canonical_dscript::CanonicalValidationFinding as DScriptValidationFinding;
use crate::core::dcompile::DCompileLoweringError;
use crate::core::dcompile_dcode::DCompileDCodeLoweringError;
use crate::core::dcompile_submission::{
    compile_submission, DCompileCandidateResponseV1, DCompileSubmissionError, DCompileSubmissionV1,
};

type ApiError = (StatusCode, JsonResponse<Value>);

fn internal_error(message: impl Into<String>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": "internal_error", "message": message.into()})),
    )
}

fn dscript_findings_json(findings: &[DScriptValidationFinding]) -> Vec<Value> {
    findings
        .iter()
        .map(|finding| json!({"code": finding.code, "message": finding.message}))
        .collect()
}

fn dgraph_findings_json(findings: &[DGraphValidationFinding]) -> Vec<Value> {
    findings
        .iter()
        .map(|finding| json!({"code": finding.code, "message": finding.message}))
        .collect()
}

fn unprocessable_dscript(error_code: &str, findings: &[DScriptValidationFinding]) -> ApiError {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        JsonResponse(json!({"error": error_code, "findings": dscript_findings_json(findings)})),
    )
}

/// Maps the existing, unmodified `DCompileLoweringError` (117C) onto HTTP --
/// every finding-carrying variant is a real, structured 422; the two
/// digest-mismatch variants and `GeneratedDGraphInvalid` are structurally
/// unreachable in the submission flow (this handler always constructs an
/// honest, internally-consistent wrapper -- see
/// `dcompile_submission::compile_submission`'s own doc comment) but are
/// still modeled as 500s rather than panicking, exactly like 117C's own
/// "theoretically unreachable, honestly modeled" precedent.
fn dgraph_lowering_error_to_api(error: DCompileLoweringError) -> ApiError {
    match error {
        DCompileLoweringError::InvalidDScript(findings) => {
            unprocessable_dscript("dcompile_invalid_dscript", &findings)
        }
        DCompileLoweringError::DScriptDigestMismatch { .. } => {
            internal_error("D-Script wrapper digest mismatch during compilation (internal inconsistency)")
        }
        DCompileLoweringError::InvalidDCompileRequest(findings) => {
            unprocessable_dscript("dcompile_invalid_request", &findings)
        }
        DCompileLoweringError::DCompileRequestDigestMismatch { .. } => {
            internal_error("D-Compile-request wrapper digest mismatch during compilation (internal inconsistency)")
        }
        DCompileLoweringError::RelationalMismatch(findings) => {
            unprocessable_dscript("dcompile_request_dscript_relational_mismatch", &findings)
        }
        DCompileLoweringError::GeneratedDGraphInvalid(findings) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            JsonResponse(json!({"error": "dcompile_generated_dgraph_invalid", "findings": dgraph_findings_json(&findings)})),
        ),
        DCompileLoweringError::DigestComputationFailed(message) => internal_error(message),
    }
}

/// Maps the existing, unmodified `DCompileDCodeLoweringError` (117E) onto
/// HTTP. `GeneratedArtifactInvalid` carries `service_id` STRUCTURALLY in the
/// JSON body (task Section 34) -- a caller never has to parse a message
/// string to learn which service's D-Code input was invalid.
fn dcode_lowering_error_to_api(error: DCompileDCodeLoweringError) -> ApiError {
    match error {
        DCompileDCodeLoweringError::DGraphLowering(inner) => dgraph_lowering_error_to_api(inner),
        DCompileDCodeLoweringError::InvalidDCodeInput(findings) => {
            unprocessable_dscript("dcompile_invalid_dcode_input", &findings)
        }
        DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings) => {
            unprocessable_dscript("dcompile_dcode_input_dgraph_relational_mismatch", &findings)
        }
        DCompileDCodeLoweringError::GeneratedArtifactInvalid { service_id, findings } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({
                "error": "dcompile_generated_artifact_invalid",
                "service_id": service_id,
                "findings": dgraph_findings_json(&findings),
            })),
        ),
        DCompileDCodeLoweringError::DigestComputationFailed(message) => internal_error(message),
    }
}

fn submission_error_to_api(error: DCompileSubmissionError) -> ApiError {
    match error {
        DCompileSubmissionError::InvalidSubmission(findings) => {
            unprocessable_dscript("dcompile_invalid_submission", &findings)
        }
        DCompileSubmissionError::InvalidDScript(findings) => {
            unprocessable_dscript("dcompile_invalid_dscript", &findings)
        }
        DCompileSubmissionError::DScriptDigestComputation(message) => internal_error(message),
        DCompileSubmissionError::InvalidDCompileRequest(findings) => {
            unprocessable_dscript("dcompile_invalid_request", &findings)
        }
        DCompileSubmissionError::DCompileRequestDigestComputation(message) => internal_error(message),
        DCompileSubmissionError::DGraphLowering(inner) => dgraph_lowering_error_to_api(inner),
        DCompileSubmissionError::DCodeLowering(inner) => dcode_lowering_error_to_api(inner),
    }
}

/// `POST /api/v1/dcompile/compile`. Pure, deterministic, side-effect-free --
/// identical semantic input always produces identical semantic output. No
/// registration, no proposal creation, no acceptance, no placement, no
/// authorization call happens inside this handler; no lock of any kind is
/// acquired. See the module doc comment and
/// `documentation/reference/phase117j-production-dcompile-api.md` for the
/// full request/response contract and example.
pub async fn post_compile(
    Json(submission): Json<DCompileSubmissionV1>,
) -> Result<JsonResponse<DCompileCandidateResponseV1>, ApiError> {
    compile_submission(submission)
        .map(JsonResponse)
        .map_err(submission_error_to_api)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dgraph::CanonicalDServResourceRequirementV1;
    use crate::core::canonical_dscript::{CanonicalDFunctionV1, CanonicalDScriptV1, CANONICAL_DSCRIPT_SCHEMA};
    use crate::core::dcompile_dcode::DCompileDCodeServiceInputV1;
    use crate::core::dcompile_request::DCompileServiceMappingV1;
    use crate::core::dcompile_submission::DCOMPILE_SUBMISSION_SCHEMA;
    use crate::core::dserv_artifact::{DServArtifactLimitsV1, DServArtifactPortsV1};

    fn valid_submission() -> DCompileSubmissionV1 {
        DCompileSubmissionV1 {
            schema: DCOMPILE_SUBMISSION_SCHEMA.to_string(),
            dscript: CanonicalDScriptV1 {
                schema: CANONICAL_DSCRIPT_SCHEMA.to_string(),
                application_id: "app:x".to_string(),
                dscript_id: "dscript:x".to_string(),
                revision: 1,
                functions: vec![CanonicalDFunctionV1 { function_id: "A".to_string(), annotations: Default::default() }],
            },
            output_dgraph_id: "dgraph:x".to_string(),
            output_dgraph_revision: 1,
            services: vec![DCompileServiceMappingV1 {
                service_id: "svc-a".to_string(),
                function_ids: vec!["A".to_string()],
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 134_217_728 },
            }],
            dcode_services: vec![DCompileDCodeServiceInputV1 {
                service_id: "svc-a".to_string(),
                artifact_id: "dserv:svc-a".to_string(),
                version: "0.1.0".to_string(),
                sha256: "a".repeat(64),
                size_bytes: 1024,
                ports: DServArtifactPortsV1::default(),
                limits: DServArtifactLimitsV1 { fuel_per_invocation: 5_000_000, timeout_ms: 200, max_memory_pages: 4, max_message_bytes: 65536, max_emits_per_invocation: 8 },
            }],
        }
    }

    /// Task Section 58: exercise the handler function directly (no new
    /// HTTP-testing framework/dependency added -- `tokio::test` is already
    /// available via this crate's own `tokio = { features = ["full"] }`
    /// dependency, needed for the server itself).
    #[tokio::test]
    async fn post_compile_returns_success_for_a_valid_submission() {
        let result = post_compile(Json(valid_submission())).await;
        assert!(result.is_ok(), "{:?}", result.err().map(|(status, _)| status));
    }

    #[tokio::test]
    async fn post_compile_returns_structured_422_for_an_invalid_submission() {
        let mut submission = valid_submission();
        submission.output_dgraph_revision = 0;
        let error = post_compile(Json(submission)).await.unwrap_err();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        let body = error.1 .0;
        assert_eq!(body["error"], "dcompile_invalid_submission");
        assert!(body["findings"].is_array());
    }

    #[tokio::test]
    async fn post_compile_includes_service_id_for_a_generated_artifact_invalid_error() {
        let mut submission = valid_submission();
        submission.dcode_services[0].limits.fuel_per_invocation = 0;
        let error = post_compile(Json(submission)).await.unwrap_err();
        assert_eq!(error.0, StatusCode::UNPROCESSABLE_ENTITY);
        let body = error.1 .0;
        assert_eq!(body["error"], "dcompile_generated_artifact_invalid");
        assert_eq!(body["service_id"], "svc-a");
    }

    /// Task Section 59: prove the production route is registered exactly
    /// once, as `POST`, by scanning `main.rs`'s own source -- mirroring
    /// `api::ddeploy.rs`'s own established static-source-scan test
    /// convention (this crate has no precedent for constructing a real
    /// `AppState`/router inside a unit test).
    #[test]
    fn production_route_is_registered_exactly_once_as_post() {
        let main_source = include_str!("../main.rs");
        let occurrences = main_source.matches("/api/v1/dcompile/compile").count();
        assert_eq!(occurrences, 1, "expected exactly one registration of the production DCompile route");
        assert!(
            main_source.contains(r#".route("/api/v1/dcompile/compile", post(post_compile))"#),
            "expected the route to be registered with the POST method against post_compile"
        );
        assert!(
            !main_source.contains(r#"get(post_compile)"#),
            "the DCompile route must never be accidentally registered as GET"
        );
    }
}
