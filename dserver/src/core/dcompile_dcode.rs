//! Phase 117E: the second D-Compile lowering step. Extends Phase 117C's pure
//! calls-empty D-Graph candidate with candidate `CanonicalDServArtifactV1`
//! (D-Code, ADR-0021) descriptors -- one per generated D-Serv, built from an
//! explicit, narrow, externally-supplied content/policy input.
//!
//! Phase 117D (`documentation/reference/phase117d-dcode-descriptor-content-identity-audit.md`)
//! established that D-Compile can never build executable bytes or compute a
//! module's content digest -- neither capability exists anywhere in this
//! repository. This module therefore ACCEPTS an already-known digest and
//! size (the only two real external facts D-Compile can never invent) and
//! deterministically constructs everything else: most `CanonicalDServArtifactV1`
//! fields are sole-legal-v0.1-value fixed constants, not policy choices
//! (117D Section 6).
//!
//! **This is candidate construction, not registration, distribution,
//! authorization, or execution.** Nothing here calls
//! `dcode_registry::declare_dserv_artifact`, touches
//! `DATUM::dcode::module_store`/`invoke`, or calls `dcode_authorization`.
//! Those remain separate, later, explicit authority transitions.
//!
//! **Composition, not reimplementation.** This module never re-derives
//! D-Script validation, D-Compile-request validation, relational validation,
//! D-Graph construction, or D-Graph digest verification -- it calls
//! `dcompile::lower_calls_empty_dgraph_candidate` exactly once and builds on
//! its trusted-for-this-call result. Any future tightening of that function
//! automatically remains effective here.
//!
//! **Content-identity trust boundary (unchanged from 117D):** D-Compile
//! accepts a DECLARED module digest and size; it does not and cannot
//! establish that they describe real bytes. That proof remains entirely
//! DATUM-side (`DCodeModuleStore::install`/`invoke_dcode`'s own
//! verification against actually-loaded bytes). This module formats/binds
//! declared content identity into the three canonical representations
//! ADR-0021 requires (`sha256`/`content_id`/`uri`) -- it never establishes
//! content truth.
//!
//! **No function-to-binary proof.** Binding a `service_id` to a declared
//! module digest proves nothing about whether that module actually
//! implements the D-Functions the D-Compile request mapped to it. The only
//! enforceable code-level proof anywhere in this architecture is: the
//! loaded bytes match the declared digest, and the module satisfies the
//! `datum-dnode/0` ABI/runtime constraints. No static source-analysis
//! provenance is invented here.
//!
//! **Coverage is a D-Compile-CANDIDATE completeness rule, not D-Graph
//! validity.** A `CanonicalDGraphV1` remains structurally valid with zero
//! services or with services that have no D-Code at all (117C already
//! proved this for the empty case, and `CanonicalDGraphV1::validate()` is
//! never touched or extended by this module). "Every generated D-Serv has
//! exactly one D-Code input" is a rule THIS bundle-construction function
//! enforces about its own completeness claim -- never a change to what
//! makes a D-Graph itself valid.
//!
//! **Server-side only**, mirroring `dcompile.rs`/`dcompile_request.rs`'s own
//! decision: `CanonicalDServArtifactV1` already has the DATUM mirror
//! execution integrity requires; this D-Compile-specific input contract has
//! no D-Node-agent consumer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dgraph::CanonicalValidationFinding as DGraphValidationFinding;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::canonical_dscript::{
    CanonicalValidationFinding as DScriptValidationFinding, ValidatedDScriptInput,
};
use crate::core::dcompile::{
    lower_calls_empty_dgraph_candidate, DCompileDGraphCandidate, DCompileLoweringError,
};
use crate::core::dcompile_request::ValidatedDCompileRequest;
use crate::core::dserv_artifact::{
    recompute_dserv_artifact_digest, CanonicalDServArtifactV1, DServArtifactAbiV1,
    DServArtifactConstraintsV1, DServArtifactDCodeV1, DServArtifactLimitsV1, DServArtifactPortsV1,
    DServArtifactSafetyV1, CANONICAL_DSERV_ARTIFACT_SCHEMA, DNODE_ABI_V0_REQUIRED_EXPORTS,
};
use crate::core::jcs;

pub const CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA: &str = "datum.dcompile-dcode-input/1";

fn finding(code: &'static str, message: impl Into<String>) -> DScriptValidationFinding {
    DScriptValidationFinding {
        code,
        message: message.into(),
    }
}

/// The explicit external content/policy input for one generated D-Serv.
/// Every field here is either a genuine external fact D-Compile can never
/// invent (`sha256`, `size_bytes`) or explicit interface/policy metadata
/// (`ports`, `limits`) that 117D found has no canonical default anywhere
/// and must never be derived from D-Graph resources, D-Script annotations,
/// or function count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileDCodeServiceInputV1 {
    /// Must exist among the bound candidate D-Graph's generated services
    /// (`validate_against_dgraph`) -- binding is by `service_id`, never by
    /// vector position, function order, `artifact_id`, or module digest.
    pub service_id: String,
    /// Descriptive identity only (117D Section 9/12): the registry keys on
    /// `(application_id, dserv_id)`, never on `artifact_id`. Not generated,
    /// not derived from `service_id`/module digest/D-Graph digest.
    pub artifact_id: String,
    /// Descriptive identity only (117D Section 9/13): the current registry
    /// ignores this for lookup/replacement -- changing it can NEVER replace
    /// an already-registered artifact for the same `(application_id,
    /// dserv_id)`. Never interpreted as a replacement/upgrade permission,
    /// currentness marker, or registry revision here or anywhere.
    pub version: String,
    /// The module's declared content digest -- EXACTLY 64 lowercase hex
    /// characters, no `sha256:` prefix (one canonical spelling only, per
    /// 117D's own recommendation, even though `CanonicalDServArtifactV1`'s
    /// own `dcode.sha256` field tolerates either). D-Compile never receives
    /// module bytes and therefore never verifies this digest describes
    /// real content -- it only formats/binds it into the three canonical
    /// representations ADR-0021 requires (Section: URI/content_id
    /// derivation below). That real-bytes proof remains entirely
    /// DATUM-side.
    pub sha256: String,
    /// The module's declared byte length. Never computed here (D-Compile
    /// has no bytes to measure) and never derived from limits, resources,
    /// or the digest.
    pub size_bytes: u64,
    /// Explicit local D-Code interface metadata (117D Section 13): NEVER
    /// derived from D-Script functions, annotations, or D-Calls, and never
    /// itself a source of D-Call topology -- ports are resolved against an
    /// ALREADY-EXISTING `CanonicalDCallV1` edge by `dcall.rs`'s own route
    /// resolution, never used to invent one. `#[serde(default)]` would let
    /// a caller omit this entirely; the field stays required-but-possibly-empty
    /// so "this module declares zero ports" remains an explicit statement
    /// rather than an accidental omission.
    pub ports: DServArtifactPortsV1,
    /// Explicit invocation-sandbox policy (117D Section 14/18): never
    /// defaulted, never derived from `CanonicalDGraphV1.resource_requirement`
    /// (a different concept entirely -- deployment capacity vs. per-invocation
    /// sandbox bound, 117D Section 17), never derived from D-Script
    /// annotations or function count. `CanonicalDServArtifactV1::validate()`
    /// remains the sole enforcer of "every limit > 0" (Section on generated
    /// artifact self-validation below) -- this input contract does not
    /// duplicate that check.
    pub limits: DServArtifactLimitsV1,
}

/// The complete, D-Graph-bound external D-Code input for one D-Compile
/// candidate. Binds itself immutably to the EXACT candidate D-Graph it was
/// prepared for via `dgraph_ref` (mirrors `dcompile_request.rs`'s own
/// `dscript_ref` pattern exactly, reusing the same genuinely project-wide
/// `CanonicalContractReferenceV1` -- not a new coupling).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCompileDCodeInputV1 {
    pub schema: String,
    /// Must exactly match the bound candidate D-Graph's `application_id`
    /// (`validate_against_dgraph`) -- never inferred or rewritten.
    pub application_id: String,
    /// `id` = the candidate D-Graph's `dgraph_id`, `revision` = its
    /// `revision`, `digest` = its freshly-recomputed digest
    /// (`DCompileDGraphCandidate::dgraph_digest_sha256`). Prevents D-Code
    /// content/policy input prepared for one D-Graph revision from being
    /// silently applied to a different one.
    pub dgraph_ref: CanonicalContractReferenceV1,
    /// One entry per generated D-Serv this D-Code input claims to cover.
    /// Order carries no meaning (mirrors D-Compile-request's own `services`
    /// set semantics); output artifact ordering is always the candidate
    /// D-Graph's own already-deterministic service order (Section on
    /// candidate construction below), never this vector's order.
    #[serde(default)]
    pub services: Vec<DCompileDCodeServiceInputV1>,
}

impl DCompileDCodeInputV1 {
    /// Pure, self-contained structural validation -- never touches a
    /// candidate D-Graph. Collects every finding; returns `Vec::new()` iff
    /// valid. Deliberately does NOT re-check `limits.* > 0` or port
    /// non-emptiness: `CanonicalDServArtifactV1::validate()` (invoked once
    /// per constructed artifact, Section below) is the sole source of truth
    /// for those -- duplicating it here would create a second, divergent
    /// definition of artifact validity.
    pub fn validate(&self) -> Vec<DScriptValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!(
                    "schema must be '{CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA}', got '{}'",
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
        if self.dgraph_ref.id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "dgraph_ref.id must be a non-empty string",
            ));
        }
        if self.dgraph_ref.revision < 1 {
            findings.push(finding(
                "revision_invalid",
                "dgraph_ref.revision must be an integer >= 1",
            ));
        }
        if !jcs::is_canonical_digest_format(&self.dgraph_ref.digest) {
            findings.push(finding(
                "dgraph_digest_malformed",
                format!(
                    "dgraph_ref.digest must be 'sha256:<64 lowercase hex>', got '{}'",
                    self.dgraph_ref.digest
                ),
            ));
        }

        let mut seen_service_ids = std::collections::HashSet::new();
        for service in &self.services {
            if service.service_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "service_id must be a non-empty string",
                ));
            } else if !seen_service_ids.insert(service.service_id.as_str()) {
                findings.push(finding(
                    "duplicate_service_input",
                    format!(
                        "duplicate DCode input for service_id '{}'",
                        service.service_id
                    ),
                ));
            }
            if service.artifact_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "artifact_id must be a non-empty string",
                ));
            }
            if service.version.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "version must be a non-empty string",
                ));
            }
            if !is_bare_sha256_hex(&service.sha256) {
                findings.push(finding(
                    "module_sha256_malformed",
                    format!(
                        "sha256 must be exactly 64 lowercase hex characters with no prefix, got '{}'",
                        service.sha256
                    ),
                ));
            }
        }

        findings
    }

    /// Relational validation against a REAL, freshly-produced
    /// `DCompileDGraphCandidate` (never a caller-supplied one -- see
    /// `lower_dcode_artifact_candidates`, which always derives this from
    /// `dcompile::lower_calls_empty_dgraph_candidate` itself). Never
    /// duplicates `DCompileDGraphCandidate`'s own construction or the
    /// canonical D-Graph validator.
    pub fn validate_against_dgraph(
        &self,
        candidate: &DCompileDGraphCandidate,
    ) -> Vec<DScriptValidationFinding> {
        let mut findings = Vec::new();

        if self.application_id != candidate.dgraph.application_id {
            findings.push(finding(
                "application_id_mismatch",
                format!(
                    "dcode input application_id '{}' does not match candidate D-Graph application_id '{}'",
                    self.application_id, candidate.dgraph.application_id
                ),
            ));
        }
        if self.dgraph_ref.id != candidate.dgraph.dgraph_id {
            findings.push(finding(
                "dgraph_id_mismatch",
                format!(
                    "dgraph_ref.id '{}' does not match candidate dgraph_id '{}'",
                    self.dgraph_ref.id, candidate.dgraph.dgraph_id
                ),
            ));
        }
        if self.dgraph_ref.revision != candidate.dgraph.revision {
            findings.push(finding(
                "dgraph_revision_mismatch",
                format!(
                    "dgraph_ref.revision {} does not match candidate revision {}",
                    self.dgraph_ref.revision, candidate.dgraph.revision
                ),
            ));
        }
        if self.dgraph_ref.digest != candidate.dgraph_digest_sha256 {
            findings.push(finding(
                "dgraph_digest_mismatch",
                format!(
                    "dgraph_ref.digest '{}' does not match the candidate D-Graph's actual digest '{}'",
                    self.dgraph_ref.digest, candidate.dgraph_digest_sha256
                ),
            ));
        }

        let candidate_service_ids: std::collections::BTreeSet<&str> = candidate
            .dgraph
            .services
            .iter()
            .map(|s| s.service_id.as_str())
            .collect();
        let input_service_ids: std::collections::BTreeSet<&str> = self
            .services
            .iter()
            .map(|s| s.service_id.as_str())
            .collect();

        for service_id in input_service_ids.difference(&candidate_service_ids) {
            findings.push(finding(
                "unknown_service_dcode_input",
                format!("DCode input references service_id '{service_id}', which does not exist in the candidate D-Graph"),
            ));
        }
        for service_id in candidate_service_ids.difference(&input_service_ids) {
            findings.push(finding(
                "missing_service_dcode_input",
                format!("candidate D-Graph service '{service_id}' has no DCode input -- this D-Compile candidate bundle is incomplete"),
            ));
        }

        findings
    }
}

fn is_bare_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Deserialization-boundary-only error (mirrors `canonical_dscript`'s own
/// two syntactic/structural classes; there is no digest to compute for this
/// type -- see the module doc's note on why no bundle-level input digest
/// exists).
#[derive(Debug)]
pub enum DCompileDCodeInputError {
    Deserialization(String),
    Validation(Vec<DScriptValidationFinding>),
}

/// Pure JSON decode + structural-validation boundary, mirroring
/// `canonical_dscript::decode_and_validate_dscript_json`'s shape exactly
/// (minus a digest -- there is nothing here worth digesting independently
/// of the artifacts this input produces; see the module doc comment).
/// Never performs relational validation against a D-Graph -- that step
/// stays explicit, exactly like `dcompile_request`'s own boundary.
pub fn decode_and_validate_dcode_input_json(
    json: &str,
) -> Result<DCompileDCodeInputV1, DCompileDCodeInputError> {
    let input: DCompileDCodeInputV1 = serde_json::from_str(json)
        .map_err(|error| DCompileDCodeInputError::Deserialization(error.to_string()))?;
    let findings = input.validate();
    if !findings.is_empty() {
        return Err(DCompileDCodeInputError::Validation(findings));
    }
    Ok(input)
}

/// Distinguishes exactly where D-Code candidate construction refused to
/// proceed. Never a single opaque `String`.
#[derive(Debug)]
pub enum DCompileDCodeLoweringError {
    /// The underlying Phase 117C D-Graph lowering itself failed -- this
    /// module never re-derives or paraphrases that failure; it reuses
    /// `DCompileLoweringError` verbatim.
    DGraphLowering(DCompileLoweringError),
    /// The D-Code input's own structural `validate()` failed.
    InvalidDCodeInput(Vec<DScriptValidationFinding>),
    /// The D-Code input structurally validates on its own, but does not
    /// relationally match the freshly-produced candidate D-Graph (wrong
    /// application/D-Graph identity/digest, unknown service, or missing
    /// coverage for a generated service).
    DCodeInputRelationalMismatch(Vec<DScriptValidationFinding>),
    /// One constructed `CanonicalDServArtifactV1` failed its own
    /// `validate()` -- carries which `service_id` and the real findings.
    /// This is the ONLY place limits/ports structural defects (e.g.
    /// `fuel_per_invocation == 0`, per the module doc comment's note on why
    /// `DCompileDCodeInputV1::validate()` does not duplicate this check)
    /// are ever caught -- proving the canonical artifact validator is
    /// actually invoked, not decorative.
    GeneratedArtifactInvalid {
        service_id: String,
        findings: Vec<DGraphValidationFinding>,
    },
    /// A descriptor digest recomputation itself failed (JCS canonicalization
    /// error) -- modeled honestly rather than unwrapped, mirroring
    /// `DCompileLoweringError::DigestComputationFailed`'s own precedent.
    DigestComputationFailed(String),
}

/// One constructed D-Code candidate: the canonical descriptor plus its
/// freshly-recomputed whole-descriptor digest. `artifact.dcode.sha256` is
/// the module CONTENT digest (declared, never verified against real bytes
/// here); `descriptor_digest_sha256` is the digest of the ENTIRE descriptor
/// -- these are deliberately different identities (117D Section 22/`dcode_authorization`'s
/// own R2A correction): a change to `limits`/`ports`/`safety`/`abi` changes
/// the descriptor digest even while the module content digest stays
/// identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DCompileDServArtifactCandidate {
    pub artifact: CanonicalDServArtifactV1,
    pub descriptor_digest_sha256: String,
}

/// The full Phase 117E result: the Phase 117C D-Graph candidate (unchanged,
/// reused verbatim) plus one D-Code artifact candidate per generated
/// service. Deliberately NOT a wire contract -- no `Serialize`/
/// `Deserialize`. Phase 117J's real production consumer
/// (`core::dcompile_submission::compile_submission`) maps this into its own
/// independently-versioned wire type (`DCompileCandidateResponseV1`)
/// instead of attaching serde here (117I Section 10, Decision B). Never
/// "accepted," "registered," or "current" merely by existing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DCompileCandidateBundle {
    pub dgraph_candidate: DCompileDGraphCandidate,
    /// Always ordered by `service_id`, matching the candidate D-Graph's own
    /// already-deterministic `services` order -- never the caller's
    /// `DCompileDCodeInputV1.services` vector order.
    pub dcode_artifacts: Vec<DCompileDServArtifactCandidate>,
}

/// The pure Phase 117E entry point. Composition, not reimplementation:
/// Step 1 delegates entirely to `dcompile::lower_calls_empty_dgraph_candidate`
/// (which itself independently revalidates `dscript`/`request` -- see that
/// function's own doc comment for the full P0 trust-boundary rationale;
/// this function inherits that defense automatically and adds none of its
/// own for the D-Graph side). No side effects: no registration, no
/// distribution, no authorization, no execution.
pub fn lower_dcode_artifact_candidates(
    dscript: &ValidatedDScriptInput,
    request: &ValidatedDCompileRequest,
    dcode_input: &DCompileDCodeInputV1,
) -> Result<DCompileCandidateBundle, DCompileDCodeLoweringError> {
    // Step 1: the ONLY D-Graph lowering path -- never reimplemented.
    let dgraph_candidate = lower_calls_empty_dgraph_candidate(dscript, request)
        .map_err(DCompileDCodeLoweringError::DGraphLowering)?;

    // Step 2: D-Code input's own structural validity.
    let input_findings = dcode_input.validate();
    if !input_findings.is_empty() {
        return Err(DCompileDCodeLoweringError::InvalidDCodeInput(
            input_findings,
        ));
    }

    // Step 3: relational agreement against the FRESHLY-PRODUCED candidate
    // from Step 1 -- never a caller-supplied candidate.
    let relational_findings = dcode_input.validate_against_dgraph(&dgraph_candidate);
    if !relational_findings.is_empty() {
        return Err(DCompileDCodeLoweringError::DCodeInputRelationalMismatch(
            relational_findings,
        ));
    }

    // Step 4: construct one artifact per generated service, in the
    // candidate D-Graph's own deterministic order (never the input
    // vector's order).
    let inputs_by_service: BTreeMap<&str, &DCompileDCodeServiceInputV1> = dcode_input
        .services
        .iter()
        .map(|service| (service.service_id.as_str(), service))
        .collect();

    let mut dcode_artifacts = Vec::with_capacity(dgraph_candidate.dgraph.services.len());
    for service in &dgraph_candidate.dgraph.services {
        // Coverage was already proven complete in Step 3; this lookup
        // cannot fail given a clean relational validation.
        let service_input = inputs_by_service.get(service.service_id.as_str()).expect(
            "validate_against_dgraph already proved every generated service has a DCode input",
        );

        let artifact = CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: service_input.artifact_id.clone(),
            version: service_input.version.clone(),
            application_id: dgraph_candidate.dgraph.application_id.clone(),
            dserv_id: service.service_id.clone(),
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{}", service_input.sha256),
                sha256: service_input.sha256.clone(),
                content_id: format!("sha256:{}", service_input.sha256),
                size_bytes: service_input.size_bytes,
            },
            abi: DServArtifactAbiV1 {
                name: "datum-dnode".to_string(),
                version: "0".to_string(),
                required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                allowed_imports: Vec::new(),
            },
            ports: service_input.ports.clone(),
            limits: service_input.limits,
            safety: DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: DServArtifactConstraintsV1::default(),
            qos: serde_json::Value::Null,
            provenance: serde_json::Value::Null,
            probes: serde_json::Value::Null,
            signature_ref: None,
        };

        // Step 5: the canonical artifact validator is the sole source of
        // truth -- never re-derived here. This is the only place a bad
        // `limits`/`ports` value (never checked by DCompileDCodeInputV1::validate())
        // is ever caught.
        let artifact_findings = artifact.validate();
        if !artifact_findings.is_empty() {
            return Err(DCompileDCodeLoweringError::GeneratedArtifactInvalid {
                service_id: service.service_id.clone(),
                findings: artifact_findings,
            });
        }

        // Step 6: the descriptor digest is always freshly computed, never
        // trusted from any input.
        let descriptor_digest_sha256 = recompute_dserv_artifact_digest(&artifact)
            .map_err(DCompileDCodeLoweringError::DigestComputationFailed)?;

        dcode_artifacts.push(DCompileDServArtifactCandidate {
            artifact,
            descriptor_digest_sha256,
        });
    }

    Ok(DCompileCandidateBundle {
        dgraph_candidate,
        dcode_artifacts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dgraph::CanonicalDServResourceRequirementV1;
    use crate::core::canonical_dscript::{
        recompute_dscript_digest, CanonicalDFunctionV1, CanonicalDScriptV1,
    };
    use crate::core::dcompile_request::{
        recompute_dcompile_request_digest, DCompileRequestV1, DCompileServiceMappingV1,
        CANONICAL_DCOMPILE_REQUEST_SCHEMA,
    };
    use crate::core::dserv_artifact::DServArtifactPortV1;
    use std::collections::BTreeMap;

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 {
            cpu_cores,
            memory_bytes,
        }
    }

    fn limits(
        fuel: u64,
        timeout_ms: u64,
        max_memory_pages: u32,
        max_message_bytes: u32,
        max_emits: u32,
    ) -> DServArtifactLimitsV1 {
        DServArtifactLimitsV1 {
            fuel_per_invocation: fuel,
            timeout_ms,
            max_memory_pages,
            max_message_bytes,
            max_emits_per_invocation: max_emits,
        }
    }

    fn default_limits() -> DServArtifactLimitsV1 {
        limits(5_000_000, 200, 64, 65536, 8)
    }

    fn dscript_doc(ids: &[&str]) -> CanonicalDScriptV1 {
        CanonicalDScriptV1 {
            schema: "datum.dscript/1".to_string(),
            application_id: "app:x".to_string(),
            dscript_id: "dscript:x".to_string(),
            revision: 1,
            functions: ids
                .iter()
                .map(|id| CanonicalDFunctionV1 {
                    function_id: id.to_string(),
                    annotations: BTreeMap::new(),
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
        mappings: &[(&str, &[&str])],
        dgraph_id: &str,
        revision: u64,
    ) -> ValidatedDCompileRequest {
        let request = DCompileRequestV1 {
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
                .map(|(service_id, function_ids)| DCompileServiceMappingV1 {
                    service_id: service_id.to_string(),
                    function_ids: function_ids.iter().map(|s| s.to_string()).collect(),
                    resource_requirement: res(1, 134_217_728),
                })
                .collect(),
        };
        let digest_sha256 = recompute_dcompile_request_digest(&request).unwrap();
        ValidatedDCompileRequest {
            request,
            digest_sha256,
        }
    }

    fn service_input(service_id: &str, sha256: &str) -> DCompileDCodeServiceInputV1 {
        DCompileDCodeServiceInputV1 {
            service_id: service_id.to_string(),
            artifact_id: format!("artifact:{service_id}"),
            version: "1".to_string(),
            sha256: sha256.to_string(),
            size_bytes: 128,
            ports: DServArtifactPortsV1::default(),
            limits: default_limits(),
        }
    }

    fn dcode_input_for(
        candidate: &DCompileDGraphCandidate,
        services: Vec<DCompileDCodeServiceInputV1>,
    ) -> DCompileDCodeInputV1 {
        DCompileDCodeInputV1 {
            schema: CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA.to_string(),
            application_id: candidate.dgraph.application_id.clone(),
            dgraph_ref: CanonicalContractReferenceV1 {
                id: candidate.dgraph.dgraph_id.clone(),
                revision: candidate.dgraph.revision,
                digest: candidate.dgraph_digest_sha256.clone(),
            },
            services,
        }
    }

    const DIGEST_A: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    fn digest(byte: char) -> String {
        std::iter::repeat(byte).take(64).collect()
    }

    // ---- E1-E3: basic construction shapes ------------------------------------ //

    #[test]
    fn e1_one_dserv_one_valid_dcode_input_produces_one_valid_artifact() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(bundle.dcode_artifacts.len(), 1);
        assert_eq!(bundle.dcode_artifacts[0].artifact.dserv_id, "svc-a");
        assert_eq!(bundle.dcode_artifacts[0].artifact.validate(), Vec::new());
    }

    #[test]
    fn e2_multiple_dservs_produce_exactly_one_artifact_each() {
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-b", &digest('b')),
            ],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(bundle.dcode_artifacts.len(), 2);
    }

    #[test]
    fn e3_empty_dgraph_produces_zero_artifacts() {
        let dscript = validated_dscript(dscript_doc(&[]));
        let request = request_for(&dscript, &[], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![]);

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(bundle.dcode_artifacts, Vec::new());
    }

    // ---- E4: deterministic ordering ------------------------------------------ //

    #[test]
    fn e4_service_input_vector_reorder_produces_identical_artifact_ordering() {
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();

        let forward = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-b", &digest('b')),
            ],
        );
        let reversed = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-b", &digest('b')),
                service_input("svc-a", &digest('a')),
            ],
        );

        let bundle_a = lower_dcode_artifact_candidates(&dscript, &request, &forward).unwrap();
        let bundle_b = lower_dcode_artifact_candidates(&dscript, &request, &reversed).unwrap();
        assert_eq!(bundle_a, bundle_b);
        let ids: Vec<&str> = bundle_a
            .dcode_artifacts
            .iter()
            .map(|c| c.artifact.dserv_id.as_str())
            .collect();
        assert_eq!(ids, vec!["svc-a", "svc-b"]);
    }

    // ---- E5-E9: content identity / field preservation ------------------------ //

    #[test]
    fn e5_module_digest_expands_into_sha256_content_id_and_uri() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input =
            dcode_input_for(&dgraph_candidate, vec![service_input("svc-a", DIGEST_A)]);

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        let artifact = &bundle.dcode_artifacts[0].artifact;
        assert_eq!(artifact.dcode.sha256, DIGEST_A);
        assert_eq!(artifact.dcode.content_id, format!("sha256:{DIGEST_A}"));
        assert_eq!(
            artifact.dcode.uri,
            format!("datum-blob://sha256/{DIGEST_A}")
        );
    }

    #[test]
    fn e6_explicit_size_bytes_preserved_exactly() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.size_bytes = 999_999;
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(bundle.dcode_artifacts[0].artifact.dcode.size_bytes, 999_999);
    }

    #[test]
    fn e7_limits_preserved_exactly() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.limits = limits(1_000, 50, 16, 4096, 2);
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(
            bundle.dcode_artifacts[0].artifact.limits,
            limits(1_000, 50, 16, 4096, 2)
        );
    }

    #[test]
    fn e8_empty_ports_preserved() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(
            bundle.dcode_artifacts[0].artifact.ports,
            DServArtifactPortsV1::default()
        );
    }

    #[test]
    fn e9_non_empty_ports_preserved() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.ports = DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: "in".to_string(),
                payload_schema: "datum://schemas/x/1".to_string(),
                external: false,
            }],
            outputs: vec![DServArtifactPortV1 {
                port: "out".to_string(),
                payload_schema: "datum://schemas/y/1".to_string(),
                external: true,
            }],
        };
        let expected_ports = input.ports.clone();
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(bundle.dcode_artifacts[0].artifact.ports, expected_ports);
    }

    // ---- E10-E12: fixed wiring + self-validation + digest -------------------- //

    #[test]
    fn e10_every_fixed_field_is_exact() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        let artifact = &bundle.dcode_artifacts[0].artifact;
        assert_eq!(artifact.schema, CANONICAL_DSERV_ARTIFACT_SCHEMA);
        assert_eq!(artifact.abi.name, "datum-dnode");
        assert_eq!(artifact.abi.version, "0");
        let expected_exports: Vec<String> = DNODE_ABI_V0_REQUIRED_EXPORTS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(artifact.abi.required_exports, expected_exports);
        assert_eq!(artifact.abi.allowed_imports, Vec::<String>::new());
        assert_eq!(artifact.safety, DServArtifactSafetyV1::default());
        assert!(
            !artifact.safety.filesystem_access
                && !artifact.safety.network_access
                && !artifact.safety.host_command_access
        );
        assert!(!artifact.safety.docker_access && !artifact.safety.side_effects_allowed);
        assert_eq!(artifact.state_model, "stateless");
        assert_eq!(artifact.instantiation, "fresh_instance_per_invocation");
        assert_eq!(artifact.constraints.allowed_stages, Vec::<String>::new());
        assert_eq!(artifact.signature_ref, None);
        assert_eq!(artifact.qos, serde_json::Value::Null);
        assert_eq!(artifact.provenance, serde_json::Value::Null);
        assert_eq!(artifact.probes, serde_json::Value::Null);
    }

    #[test]
    fn e11_every_generated_artifact_validates_cleanly() {
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-b", &digest('b')),
            ],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        for candidate in &bundle.dcode_artifacts {
            assert_eq!(candidate.artifact.validate(), Vec::new());
        }
    }

    #[test]
    fn e12_descriptor_digest_matches_independent_recomputation() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        let candidate = &bundle.dcode_artifacts[0];
        assert_eq!(
            candidate.descriptor_digest_sha256,
            recompute_dserv_artifact_digest(&candidate.artifact).unwrap()
        );
    }

    // ---- E13-E14: five-identity independence --------------------------------- //

    #[test]
    fn e13_same_module_bytes_different_limits_same_module_digest_different_descriptor_digest() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();

        let mut input_a = service_input("svc-a", &digest('a'));
        input_a.limits = limits(1_000, 50, 16, 4096, 2);
        let mut input_b = service_input("svc-a", &digest('a'));
        input_b.limits = limits(2_000, 50, 16, 4096, 2);

        let bundle_a = lower_dcode_artifact_candidates(
            &dscript,
            &request,
            &dcode_input_for(&dgraph_candidate, vec![input_a]),
        )
        .unwrap();
        let bundle_b = lower_dcode_artifact_candidates(
            &dscript,
            &request,
            &dcode_input_for(&dgraph_candidate, vec![input_b]),
        )
        .unwrap();

        assert_eq!(
            bundle_a.dcode_artifacts[0].artifact.dcode.sha256,
            bundle_b.dcode_artifacts[0].artifact.dcode.sha256
        );
        assert_ne!(
            bundle_a.dcode_artifacts[0].descriptor_digest_sha256,
            bundle_b.dcode_artifacts[0].descriptor_digest_sha256
        );
    }

    #[test]
    fn e14_same_bytes_different_service_binding_same_module_digest_different_descriptor_digest() {
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-b", &digest('a')),
            ],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(
            bundle.dcode_artifacts[0].artifact.dcode.sha256,
            bundle.dcode_artifacts[1].artifact.dcode.sha256
        );
        assert_ne!(
            bundle.dcode_artifacts[0].descriptor_digest_sha256,
            bundle.dcode_artifacts[1].descriptor_digest_sha256
        );
    }

    // ---- E15: DGraph remains unchanged/calls-empty ---------------------------- //

    #[test]
    fn e15_dgraph_candidate_remains_calls_empty_and_unchanged() {
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate_direct =
            lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate_direct,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-b", &digest('b')),
            ],
        );

        let bundle = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap();
        assert_eq!(bundle.dgraph_candidate, dgraph_candidate_direct);
        assert!(bundle.dgraph_candidate.dgraph.calls.is_empty());
    }

    // ---- E16: annotations remain irrelevant ----------------------------------- //

    #[test]
    fn e16_annotation_change_does_not_become_artifact_policy() {
        let mut annotated_doc = dscript_doc(&["A"]);
        annotated_doc.functions[0]
            .annotations
            .insert("cpu".to_string(), "99".to_string());
        let plain = validated_dscript(dscript_doc(&["A"]));
        let annotated = validated_dscript(annotated_doc);
        assert_ne!(plain.digest_sha256, annotated.digest_sha256);

        let request_plain = request_for(&plain, &[("svc-a", &["A"])], "dgraph:x", 1);
        let request_annotated = request_for(&annotated, &[("svc-a", &["A"])], "dgraph:x", 1);
        let candidate_plain = lower_calls_empty_dgraph_candidate(&plain, &request_plain).unwrap();
        let candidate_annotated =
            lower_calls_empty_dgraph_candidate(&annotated, &request_annotated).unwrap();

        let input_plain =
            dcode_input_for(&candidate_plain, vec![service_input("svc-a", &digest('a'))]);
        let input_annotated = dcode_input_for(
            &candidate_annotated,
            vec![service_input("svc-a", &digest('a'))],
        );

        let bundle_plain =
            lower_dcode_artifact_candidates(&plain, &request_plain, &input_plain).unwrap();
        let bundle_annotated =
            lower_dcode_artifact_candidates(&annotated, &request_annotated, &input_annotated)
                .unwrap();
        assert_eq!(
            bundle_plain.dcode_artifacts,
            bundle_annotated.dcode_artifacts
        );
    }

    // ---- E17-E22: structural / relational negatives on the input bundle ------ //

    #[test]
    fn e17_malformed_schema_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );
        dcode_input.schema = "not-a-real-schema".to_string();

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::InvalidDCodeInput(findings) => {
                assert!(findings.iter().any(|f| f.code == "schema_invalid"))
            }
            other => panic!("expected InvalidDCodeInput, got {other:?}"),
        }
    }

    #[test]
    fn e18_blank_application_id_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );
        dcode_input.application_id = "  ".to_string();

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        assert!(matches!(
            error,
            DCompileDCodeLoweringError::InvalidDCodeInput(_)
        ));
    }

    #[test]
    fn e19_malformed_dgraph_reference_digest_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );
        dcode_input.dgraph_ref.digest = "not-a-digest".to_string();

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::InvalidDCodeInput(findings) => {
                assert!(findings.iter().any(|f| f.code == "dgraph_digest_malformed"))
            }
            other => panic!("expected InvalidDCodeInput, got {other:?}"),
        }
    }

    #[test]
    fn e20_dgraph_reference_id_mismatch_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );
        dcode_input.dgraph_ref.id = "dgraph:someone-else".to_string();

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings) => {
                assert!(findings.iter().any(|f| f.code == "dgraph_id_mismatch"))
            }
            other => panic!("expected DCodeInputRelationalMismatch, got {other:?}"),
        }
    }

    #[test]
    fn e21_dgraph_reference_revision_mismatch_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );
        dcode_input.dgraph_ref.revision = 99;

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings) => {
                assert!(findings
                    .iter()
                    .any(|f| f.code == "dgraph_revision_mismatch"))
            }
            other => panic!("expected DCodeInputRelationalMismatch, got {other:?}"),
        }
    }

    #[test]
    fn e22_dgraph_reference_digest_mismatch_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );
        dcode_input.dgraph_ref.digest = format!("sha256:{}", "0".repeat(64));

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings) => {
                assert!(findings.iter().any(|f| f.code == "dgraph_digest_mismatch"))
            }
            other => panic!("expected DCodeInputRelationalMismatch, got {other:?}"),
        }
    }

    // ---- E23-E26: coverage ----------------------------------------------------- //

    #[test]
    fn e23_duplicate_service_input_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-a", &digest('b')),
            ],
        );

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::InvalidDCodeInput(findings) => {
                assert!(findings.iter().any(|f| f.code == "duplicate_service_input"))
            }
            other => panic!("expected InvalidDCodeInput, got {other:?}"),
        }
    }

    #[test]
    fn e24_unknown_service_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![
                service_input("svc-a", &digest('a')),
                service_input("svc-unknown", &digest('b')),
            ],
        );

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings) => {
                assert!(findings
                    .iter()
                    .any(|f| f.code == "unknown_service_dcode_input"))
            }
            other => panic!("expected DCodeInputRelationalMismatch, got {other:?}"),
        }
    }

    #[test]
    fn e25_missing_service_input_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &digest('a'))],
        );

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::DCodeInputRelationalMismatch(findings) => {
                assert!(findings
                    .iter()
                    .any(|f| f.code == "missing_service_dcode_input"))
            }
            other => panic!("expected DCodeInputRelationalMismatch, got {other:?}"),
        }
    }

    #[test]
    fn e26_blank_service_id_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.service_id = "  ".to_string();
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::InvalidDCodeInput(findings) => {
                assert!(findings.iter().any(|f| f.code == "missing_field"))
            }
            other => panic!("expected InvalidDCodeInput, got {other:?}"),
        }
    }

    // ---- E27-E29: field-level structural negatives ----------------------------- //

    #[test]
    fn e27_blank_artifact_id_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.artifact_id = "".to_string();
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        assert!(matches!(
            error,
            DCompileDCodeLoweringError::InvalidDCodeInput(_)
        ));
    }

    #[test]
    fn e28_blank_version_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.version = "".to_string();
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        assert!(matches!(
            error,
            DCompileDCodeLoweringError::InvalidDCodeInput(_)
        ));
    }

    #[test]
    fn e29_malformed_module_sha256_rejected() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        // Prefixed spelling is deliberately rejected -- one canonical
        // spelling only (bare hex), per 117D's own recommendation, even
        // though the underlying artifact field itself would tolerate a
        // "sha256:" prefix.
        let dcode_input = dcode_input_for(
            &dgraph_candidate,
            vec![service_input("svc-a", &format!("sha256:{}", digest('a')))],
        );

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::InvalidDCodeInput(findings) => {
                assert!(findings.iter().any(|f| f.code == "module_sha256_malformed"))
            }
            other => panic!("expected InvalidDCodeInput, got {other:?}"),
        }
    }

    // ---- E30-E31: caught only by the final canonical artifact validator ------- //

    /// E30: `DCompileDCodeInputV1::validate()` deliberately does not
    /// duplicate the `limits.* > 0` check -- proves it is still enforced,
    /// by the canonical artifact validator, at construction time.
    #[test]
    fn e30_zero_limit_value_caught_only_by_generated_artifact_validation() {
        let dscript = validated_dscript(dscript_doc(&["A"]));
        let request = request_for(&dscript, &[("svc-a", &["A"])], "dgraph:x", 1);
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let mut input = service_input("svc-a", &digest('a'));
        input.limits.fuel_per_invocation = 0;
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![input]);

        // Structural validation alone does not catch this.
        assert_eq!(dcode_input.validate(), Vec::new());

        let error = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input).unwrap_err();
        match error {
            DCompileDCodeLoweringError::GeneratedArtifactInvalid {
                service_id,
                findings,
            } => {
                assert_eq!(service_id, "svc-a");
                assert!(findings.iter().any(|f| f.code == "limits_invalid"));
            }
            other => panic!("expected GeneratedArtifactInvalid, got {other:?}"),
        }
    }

    // ---- E32-E36: unknown/forbidden fields, via the JSON boundary -------------- //

    fn minimal_valid_input_json() -> String {
        let sha = digest('a');
        format!(
            r#"{{"schema":"datum.dcompile-dcode-input/1","application_id":"app:x","dgraph_ref":{{"id":"dgraph:x","revision":1,"digest":"sha256:{}"}},"services":[{{"service_id":"svc-a","artifact_id":"artifact:svc-a","version":"1","sha256":"{sha}","size_bytes":128,"ports":{{"inputs":[],"outputs":[]}},"limits":{{"fuel_per_invocation":5000000,"timeout_ms":200,"max_memory_pages":64,"max_message_bytes":65536,"max_emits_per_invocation":8}}}}]}}"#,
            "0".repeat(64)
        )
    }

    #[test]
    fn e32_unknown_top_level_field_rejected() {
        let json = minimal_valid_input_json()
            .replace("\"services\":", "\"node_id\":\"fog-01\",\"services\":");
        let error = decode_and_validate_dcode_input_json(&json).unwrap_err();
        assert!(matches!(error, DCompileDCodeInputError::Deserialization(_)));
    }

    #[test]
    fn e33_caller_supplied_dcode_uri_rejected() {
        let sha = digest('a');
        let json = format!(
            r#"{{"schema":"datum.dcompile-dcode-input/1","application_id":"app:x","dgraph_ref":{{"id":"dgraph:x","revision":1,"digest":"sha256:{}"}},"services":[{{"service_id":"svc-a","artifact_id":"a","version":"1","sha256":"{sha}","size_bytes":128,"dcode_uri":"datum-blob://sha256/{sha}","ports":{{"inputs":[],"outputs":[]}},"limits":{{"fuel_per_invocation":1,"timeout_ms":1,"max_memory_pages":1,"max_message_bytes":1,"max_emits_per_invocation":1}}}}]}}"#,
            "0".repeat(64)
        );
        let error = decode_and_validate_dcode_input_json(&json).unwrap_err();
        assert!(matches!(error, DCompileDCodeInputError::Deserialization(_)));
    }

    #[test]
    fn e34_caller_supplied_content_id_rejected() {
        let sha = digest('a');
        let json = format!(
            r#"{{"schema":"datum.dcompile-dcode-input/1","application_id":"app:x","dgraph_ref":{{"id":"dgraph:x","revision":1,"digest":"sha256:{}"}},"services":[{{"service_id":"svc-a","artifact_id":"a","version":"1","sha256":"{sha}","size_bytes":128,"content_id":"sha256:{sha}","ports":{{"inputs":[],"outputs":[]}},"limits":{{"fuel_per_invocation":1,"timeout_ms":1,"max_memory_pages":1,"max_message_bytes":1,"max_emits_per_invocation":1}}}}]}}"#,
            "0".repeat(64)
        );
        let error = decode_and_validate_dcode_input_json(&json).unwrap_err();
        assert!(matches!(error, DCompileDCodeInputError::Deserialization(_)));
    }

    #[test]
    fn e35_caller_supplied_abi_selector_rejected() {
        let json = minimal_valid_input_json().replace(
            "\"schema\":\"datum.dcompile-dcode-input/1\"",
            "\"schema\":\"datum.dcompile-dcode-input/1\",\"abi\":{\"name\":\"other\"}",
        );
        let error = decode_and_validate_dcode_input_json(&json).unwrap_err();
        assert!(matches!(error, DCompileDCodeInputError::Deserialization(_)));
    }

    #[test]
    fn e36_caller_supplied_placement_node_id_rejected() {
        let json = minimal_valid_input_json().replace(
            "\"schema\":\"datum.dcompile-dcode-input/1\"",
            "\"schema\":\"datum.dcompile-dcode-input/1\",\"node_id\":\"fog-01\"",
        );
        let error = decode_and_validate_dcode_input_json(&json).unwrap_err();
        assert!(matches!(error, DCompileDCodeInputError::Deserialization(_)));
    }

    // ---- E37: forged original inputs still fail through the reused 117C path -- //

    #[test]
    fn e37_forged_dscript_wrapper_fails_through_reused_117c_path() {
        let document = dscript_doc(&["A"]);
        let forged = ValidatedDScriptInput {
            document,
            digest_sha256: "sha256:forged00000000000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let request = request_for(&forged, &[("svc-a", &["A"])], "dgraph:x", 1);
        // No candidate can even be produced, so a minimal (structurally
        // unvalidated-against-anything) input is enough to prove the error
        // originates from Step 1, before this module's own logic runs.
        let dcode_input = DCompileDCodeInputV1 {
            schema: CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dgraph_ref: CanonicalContractReferenceV1 {
                id: "dgraph:x".to_string(),
                revision: 1,
                digest: format!("sha256:{}", "0".repeat(64)),
            },
            services: vec![],
        };

        let error = lower_dcode_artifact_candidates(&forged, &request, &dcode_input).unwrap_err();
        assert!(
            matches!(
                error,
                DCompileDCodeLoweringError::DGraphLowering(
                    DCompileLoweringError::DScriptDigestMismatch { .. }
                )
            ),
            "{error:?}"
        );
    }

    // ---- E38: generated artifact validation is never silently ignored --------- //

    #[test]
    fn e38_generated_artifact_validation_failure_is_never_silently_ignored() {
        // Same scenario as E30, phrased to explicitly prove the error is
        // surfaced (not swallowed) and the bundle is never partially
        // returned on failure.
        let dscript = validated_dscript(dscript_doc(&["A", "B"]));
        let request = request_for(
            &dscript,
            &[("svc-a", &["A"]), ("svc-b", &["B"])],
            "dgraph:x",
            1,
        );
        let dgraph_candidate = lower_calls_empty_dgraph_candidate(&dscript, &request).unwrap();
        let good = service_input("svc-a", &digest('a'));
        let mut bad = service_input("svc-b", &digest('b'));
        bad.limits.max_memory_pages = 0;
        let dcode_input = dcode_input_for(&dgraph_candidate, vec![good, bad]);

        let result = lower_dcode_artifact_candidates(&dscript, &request, &dcode_input);
        assert!(result.is_err(), "a bad artifact anywhere in the bundle must fail the whole construction, never partially succeed");
    }
}
