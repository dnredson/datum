//! Top-level canonical D-Code invocation entrypoint (ADR-0021, 111.11):
//! `invoke_dcode(executor, module_store, authorization, artifact,
//! this_node_id, input_port, input)`. No D-Call is involved -- input is
//! supplied directly by the caller (CLI, test, or later a D-Call producer).
//! This function performs zero host mutation/reconciliation and never
//! contacts DServer itself (the GOVERNED CLI path calls `dcode::client`
//! immediately before invoking this function -- see
//! `control-plane-preflight-audit.txt`).
//!
//! Independent review v2 corrections integrated here:
//! - **R2**: `artifact` is now the FULL `dcode::dserv_artifact::CanonicalDServArtifactV1`
//!   mirror (not the narrow v1 `DServArtifactForInvocation` subset), so the
//!   canonical descriptor digest (`recompute_dserv_artifact_digest`) can
//!   actually be recomputed and checked against
//!   `authorization.dserv_artifact_descriptor_digest` -- the module content
//!   digest (`dserv_artifact_digest`) and the whole-descriptor digest are
//!   checked SEPARATELY, exactly mirroring the dserver-side R2A binding.
//! - **R3**: the module's declared `dcode.size_bytes` is checked against the
//!   ACTUAL loaded module byte length before execution.
//! - **R4**: input/output now flow through the ADR-0003 `datum-dnode/0`
//!   envelope (`dcode::envelope`) -- port validation, host-owned
//!   message_id/correlation_id, output status/emit validation.
//! - **R5**: execution itself is delegated to a `DCodeExecutor` -- the
//!   GOVERNED production path always uses `IsolatedWorkerExecutor`
//!   (`dcode::worker`), which runs the actual Wasmtime engine in a separate
//!   child process so a native crash there cannot take down this process.
//!   `InProcessExecutor` still exists for fast unit testing of the
//!   surrounding governance logic in this file; the REAL crash-containment
//!   proof lives in `tests/dcode_worker_isolation.rs` (integration test,
//!   exercising the real worker binary as a real subprocess).
//!
//! Documented scope boundary (ADR-0021, `stale-authority-audit.txt`): this
//! entrypoint does NOT itself re-contact DServer to call
//! `verify_dcode_authorization_still_current` before executing -- the
//! GOVERNED CLI path achieves the same effect by fetching a FRESH
//! authorization from `POST /api/v1/dcode/authorize` immediately before
//! calling this function (that endpoint always freshly derives, never
//! caches). This function itself still performs every check possible from
//! purely local data: that the artifact's identity (application/service/
//! artifact id, module digest, descriptor digest, ABI) matches the
//! authorization exactly, and that the module's declared size matches its
//! real bytes.
//!
//! `this_node_id` and "authorization targets THIS D-Node runtime" (independent
//! review v4, G1/G4 precision correction): prior to G1, an earlier version of
//! this comment's claim that the equality check below proves "the
//! authorization targets THIS node" was NOT actually true -- `this_node_id`
//! was, at the time, whatever raw value a CLI caller supplied on the command
//! line, and that SAME value was also what got sent to DServer as the node
//! being authorized. The check was circular: it proved "DServer authorized
//! the node ID the caller asked it to," never "this process really is that
//! node." G1 fixes the CALLER (the governed CLI, `smartsentinel-dcode-invoke
//! invoke`) to resolve `this_node_id` exclusively from this installation's
//! own local D-Node identity file (`dcode::local_identity`) -- never from a
//! command-line override -- and to use that SAME resolved value for both the
//! DServer authorization request and this function's local equality check.
//! With that fix in place, this sentence is now honestly true: "this
//! function proves the authorization targets THIS D-Node runtime." The local
//! trust boundary this now rests on, stated explicitly: DServer proves
//! canonical placement/authorization (which node the accepted D-Map says a
//! service belongs on); the LOCAL D-Node runtime identity file proves which
//! logical node THIS PROCESS is; local installation integrity is TRUSTED --
//! v0.1 does not defend against an attacker who already has permission to
//! replace the SmartSentinel/D-Code binaries on this host or rewrite the
//! local identity file directly (that is a different, much stronger threat
//! model requiring e.g. TPM/secure-boot/remote-attestation, which this slice
//! does not claim to provide). `offline-test-only` deliberately opts back
//! into caller-supplied `node_id` for isolated testing; it is never the
//! governed path.

use std::path::Path;
use std::time::Instant;

use crate::dcode::authorization::DCodeInvocationAuthorizationV1;
use crate::dcode::dserv_artifact::{recompute_dserv_artifact_digest, CanonicalDServArtifactV1};
use crate::dcode::envelope::{self, EnvelopeError};
use crate::dcode::evidence::{
    DCodeAppliedLimits, DCodeEmitEvidenceV1, DCodeInvocationEvidenceV1, DCodeInvocationOutcome, DCODE_EVIDENCE_SCHEMA,
};
use crate::dcode::identity::sha256_hex;
use crate::dcode::module_store::DCodeModuleStore;
use crate::dcode::runtime::{DCodeModuleCache, ExecutionOutcome, ExecutionOutput, InvocationLimits};

/// Executes one D-Code invocation given a digest, limits, and raw (already
/// envelope-wrapped) input bytes. `InProcessExecutor` is for fast unit
/// testing only; `dcode::worker::IsolatedWorkerExecutor` (used by the
/// governed production path) is the real crash-containment boundary (R5).
pub trait DCodeExecutor {
    fn execute(&self, digest: &str, limits: &InvocationLimits, input: &[u8]) -> ExecutionOutcome;
}

/// In-process execution -- fast, but offers NO crash isolation. Test-only:
/// never used by the real CLI binaries (those always use
/// `dcode::worker::IsolatedWorkerExecutor`).
pub struct InProcessExecutor<'a> {
    pub cache: &'a DCodeModuleCache,
    pub store: &'a DCodeModuleStore,
}

impl DCodeExecutor for InProcessExecutor<'_> {
    fn execute(&self, digest: &str, limits: &InvocationLimits, input: &[u8]) -> ExecutionOutcome {
        match self.store.load(digest) {
            Ok(bytes) => self.cache.invoke(digest, &bytes, limits, input),
            Err(error) => ExecutionOutcome::ValidationFailed(error),
        }
    }
}

/// Independent review fix v2 (I1A): one validated, decoded emit from a
/// successful invocation -- populated ONLY after `parse_and_validate_output`
/// succeeds (an invalid/undeclared/excess emit never reaches here). This is
/// deliberately NEVER `Serialize`/`Deserialize` and NEVER persisted -- the
/// raw emit payload exists only transiently in this trusted process's
/// memory, exactly long enough for `dcall::delivery` to dispatch it. Evidence
/// (`DCodeInvocationEvidenceV1::emits`, `DCodeEmitEvidenceV1`) remains
/// digest/size-only, unaffected by this type's existence.
///
/// Independent review fix v3 (J1A): `envelope::ValidatedEmit` itself now has
/// private fields and no public constructor (see that type's doc comment) --
/// this alias inherits that opacity, so a `ValidatedDCodeEmit` can only ever
/// be one produced by a real `parse_and_validate_output` call, accessed via
/// its `.port()`/`.payload()` methods.
pub type ValidatedDCodeEmit = crate::dcode::envelope::ValidatedEmit;

/// Independent review fix v2 (I1A): the richer, transient (never serialized)
/// result of an invocation -- `.evidence()` is identical to what
/// `invoke_dcode` alone would have returned; `.validated_emits()`
/// additionally carries the REAL decoded emit bytes for a trusted host-side
/// caller (D-Call dispatch) to consume directly, without ever persisting
/// them. Empty whenever the invocation didn't reach a successful,
/// emit-bearing output (failure, or a canonical no-output `Success`).
///
/// Independent review fix v3 (J1A): fields are PRIVATE with read-only
/// accessors, and this type does NOT implement `Deserialize` -- the only
/// place a `DCodeInvocationResult` can be constructed is inside this module's
/// own `invoke_dcode_impl`, after a real invocation actually ran. A caller
/// (in this crate or, if this library were ever depended on externally, any
/// other crate) cannot fabricate one from an arbitrary
/// `DCodeInvocationEvidenceV1` + arbitrary bytes -- see
/// `source-dcode-emit-provenance-audit.txt`.
pub struct DCodeInvocationResult {
    evidence: DCodeInvocationEvidenceV1,
    validated_emits: Vec<ValidatedDCodeEmit>,
}

impl DCodeInvocationResult {
    pub fn evidence(&self) -> &DCodeInvocationEvidenceV1 {
        &self.evidence
    }

    pub fn validated_emits(&self) -> &[ValidatedDCodeEmit] {
        &self.validated_emits
    }
}

/// Independent review fix v5 (K1A): distinguishes a result KNOWN to have
/// been produced by the real GOVERNED isolated-worker execution path from
/// the merely-opaque `DCodeInvocationResult` above. J1's opacity fix (private
/// fields, no `Deserialize`) closed direct fabrication, but it did NOT close
/// a subtler gap: `invoke_dcode_capturing_emits` still accepts any `&dyn
/// DCodeExecutor`, so a caller-implemented `FakeExecutor` returning a
/// syntactically valid ADR-0003 success envelope could still legitimately
/// mint a `DCodeInvocationResult` -- without ever running real isolated-worker
/// Wasmtime execution. `GovernedDCodeInvocationResult` has private fields, no
/// public constructor, and is never `Deserialize`; the ONLY way to obtain one
/// is `invoke_dcode_governed_capturing_emits` below (or, for a `cargo test`
/// binary that cannot rely on sibling-executable resolution / a provisioned
/// local identity file, the `dcall-test-support`-feature-gated
/// `invoke_dcode_governed_capturing_emits_for_test`) -- the production producer always runs the real isolated worker; the
/// non-default test-support producer permits synthetic executors. Production
/// D-Call dispatch (`dcall::delivery::dispatch_validated_emit`) accepts ONLY
/// this type -- see `governed-source-invocation-provenance-audit.txt`.
pub struct GovernedDCodeInvocationResult {
    inner: DCodeInvocationResult,
}

impl GovernedDCodeInvocationResult {
    pub fn evidence(&self) -> &DCodeInvocationEvidenceV1 {
        self.inner.evidence()
    }

    pub fn validated_emits(&self) -> &[ValidatedDCodeEmit] {
        self.inner.validated_emits()
    }
}

/// Independent review fix v5 (K1B): the ONLY production way to obtain a
/// `GovernedDCodeInvocationResult`. Resolves THIS installation's own local
/// D-Node identity from `dnode_root` itself (never a caller-supplied
/// `node_id` override -- mirrors the G1 fix already applied to the governed
/// D-Code CLI) and the trusted installed sibling `smartsentinel-dcode-worker`
/// binary itself via `dcode::worker::trusted_sibling_worker_binary_path`
/// (never a caller-supplied path override -- mirrors G2), then runs the real
/// Phase-111 isolated-worker execution path. Authorization/artifact are still
/// supplied by the caller because Phase-111 already validates their exact
/// binding to the resolved node identity inside `invoke_dcode_impl` -- this
/// function does not duplicate that.
pub fn invoke_dcode_governed_capturing_emits(
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    dnode_root: &Path,
    input_port: &str,
    input: &[u8],
) -> Result<GovernedDCodeInvocationResult, String> {
    let local_identity = crate::dcode::local_identity::load_local_identity(dnode_root)?;
    let worker_binary = crate::dcode::worker::trusted_sibling_worker_binary_path();
    let executor = crate::dcode::worker::IsolatedWorkerExecutor { worker_binary: &worker_binary, module_store_root: dnode_root };
    let inner = invoke_dcode_capturing_emits(&executor, module_store, authorization, artifact, &local_identity.node_id, input_port, input);
    Ok(GovernedDCodeInvocationResult { inner })
}

/// TEST-ONLY (independent review fix v5, K1D): compiled ONLY behind the
/// non-default `dcall-test-support` feature, so a normal `cargo build`/
/// `cargo build --release` never includes this symbol at all. Identical to
/// `invoke_dcode_governed_capturing_emits` except a `cargo test` binary
/// supplies its own executor/worker-resolved-`this_node_id` directly, since
/// it cannot rely on sibling-executable resolution or a provisioned local
/// identity file the way a real installation can. Still always wraps a REAL
/// `invoke_dcode_capturing_emits` call -- callers typically pass a real
/// `dcode::worker::IsolatedWorkerExecutor` here too, for the strongest test
/// evidence, though `InProcessExecutor` remains available for tests that only
/// need to exercise the surrounding governance logic quickly.
#[cfg(feature = "dcall-test-support")]
pub fn invoke_dcode_governed_capturing_emits_for_test(
    executor: &dyn DCodeExecutor,
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    this_node_id: &str,
    input_port: &str,
    input: &[u8],
) -> GovernedDCodeInvocationResult {
    let inner = invoke_dcode_capturing_emits(executor, module_store, authorization, artifact, this_node_id, input_port, input);
    GovernedDCodeInvocationResult { inner }
}

pub fn invoke_dcode(
    executor: &dyn DCodeExecutor,
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    this_node_id: &str,
    input_port: &str,
    input: &[u8],
) -> DCodeInvocationEvidenceV1 {
    invoke_dcode_impl(executor, module_store, authorization, artifact, this_node_id, input_port, input, None, None).evidence
}

/// Independent review fix v2 (I1A/I1B): identical to `invoke_dcode` except it
/// ALSO returns the real, decoded, validated emit payloads (transiently, in
/// process memory only -- never serialized/persisted). This is the ONLY
/// function that gives a caller access to real emit bytes; it exists
/// specifically so `dcall::delivery::dispatch_validated_emit` can prove a
/// dispatched payload actually came from a real successful source D-Code
/// invocation (SHA-256(payload) checked against the corresponding
/// `DCodeEmitEvidenceV1` digest already in `evidence.emits`) rather than
/// trusting arbitrary caller-supplied bytes.
pub fn invoke_dcode_capturing_emits(
    executor: &dyn DCodeExecutor,
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    this_node_id: &str,
    input_port: &str,
    input: &[u8],
) -> DCodeInvocationResult {
    invoke_dcode_impl(executor, module_store, authorization, artifact, this_node_id, input_port, input, None, None)
}

/// Independent review fix v2 (I2A): identical to `invoke_dcode` except BOTH
/// `message_id` AND `correlation_id` are host-supplied rather than minted
/// here -- used EXCLUSIVELY by `dcall::delivery`'s destination execution, so
/// a D-Call delivery grant's own bound `message_id`/`correlation_id` are the
/// SAME identifiers that end up in both the guest-visible input envelope and
/// this invocation's own `DCodeInvocationEvidenceV1`, closing the gap where
/// the grant and the actual destination invocation could otherwise mint two
/// independent, potentially-divergent message identities for what is
/// supposed to be one delivery. Never exposed to a raw CLI/API caller.
pub fn invoke_dcode_with_delivery_identity(
    executor: &dyn DCodeExecutor,
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    this_node_id: &str,
    input_port: &str,
    input: &[u8],
    message_id: &str,
    correlation_id: &str,
) -> DCodeInvocationEvidenceV1 {
    invoke_dcode_impl(
        executor,
        module_store,
        authorization,
        artifact,
        this_node_id,
        input_port,
        input,
        None,
        Some((message_id.to_string(), correlation_id.to_string())),
    )
    .evidence
}

/// Phase 112 (ADR-0022): identical to `invoke_dcode` except the envelope's
/// `correlation_id` is the caller-supplied value (propagated by the D-Call
/// host layer across one logical call chain) instead of a freshly minted
/// one -- `message_id` is still always freshly generated per invocation,
/// exactly as `invoke_dcode` does. Never exposed to a raw CLI/API caller;
/// only `dcall::delivery`'s host-mediated dispatch uses this.
pub fn invoke_dcode_with_correlation(
    executor: &dyn DCodeExecutor,
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    this_node_id: &str,
    input_port: &str,
    input: &[u8],
    correlation_id: &str,
) -> DCodeInvocationEvidenceV1 {
    invoke_dcode_impl(
        executor,
        module_store,
        authorization,
        artifact,
        this_node_id,
        input_port,
        input,
        Some(correlation_id.to_string()),
        None,
    )
    .evidence
}

fn invoke_dcode_impl(
    executor: &dyn DCodeExecutor,
    module_store: &DCodeModuleStore,
    authorization: &DCodeInvocationAuthorizationV1,
    artifact: &CanonicalDServArtifactV1,
    this_node_id: &str,
    input_port: &str,
    input: &[u8],
    correlation_override: Option<String>,
    identity_override: Option<(String, String)>,
) -> DCodeInvocationResult {
    let started = Instant::now();
    let input_sha256 = sha256_hex(input);
    let limits_applied = DCodeAppliedLimits {
        fuel_per_invocation: artifact.limits.fuel_per_invocation,
        timeout_ms: artifact.limits.timeout_ms,
        max_memory_pages: artifact.limits.max_memory_pages,
        max_message_bytes: artifact.limits.max_message_bytes,
    };

    let mut evidence = DCodeInvocationEvidenceV1 {
        schema: DCODE_EVIDENCE_SCHEMA.to_string(),
        authorization_id: authorization.authorization_id.clone(),
        project_id: authorization.project_id.clone(),
        application_id: authorization.application_id.clone(),
        service_id: authorization.service_id.clone(),
        node_id: authorization.node_id.clone(),
        dmap_id: authorization.dmap_ref.id.clone(),
        dmap_revision: authorization.dmap_ref.revision,
        dmap_digest: authorization.dmap_ref.digest.clone(),
        dgraph_id: authorization.dgraph_ref.id.clone(),
        dgraph_revision: authorization.dgraph_ref.revision,
        dgraph_digest: authorization.dgraph_ref.digest.clone(),
        dserv_artifact_id: authorization.dserv_artifact_id.clone(),
        dserv_artifact_digest: authorization.dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: authorization.dserv_artifact_descriptor_digest.clone(),
        module_size_bytes: None,
        dnode_abi: authorization.dnode_abi.clone(),
        fresh_instance_per_invocation: true,
        input_port: input_port.to_string(),
        message_id: String::new(),
        correlation_id: String::new(),
        input_sha256: input_sha256.clone(),
        input_size: input.len(),
        input_envelope_sha256: String::new(),
        input_envelope_size: 0,
        received_at_utc: String::new(),
        output_sha256: None,
        output_size: None,
        output_status: None,
        output_reason_present: false,
        output_reason_sha256: None,
        output_reason_size: None,
        emits: Vec::new(),
        limits_applied,
        outcome: DCodeInvocationOutcome::ValidationFailed,
        error_code: None,
        error_message: None,
        error_offending_value_sha256: None,
        error_offending_value_size: None,
        duration_ms: 0,
    };

    macro_rules! fail {
        ($outcome:expr, $code:expr, $message:expr) => {{
            evidence.outcome = $outcome;
            evidence.error_code = Some($code.to_string());
            evidence.error_message = Some($message);
            evidence.duration_ms = started.elapsed().as_millis() as u64;
            return DCodeInvocationResult { evidence, validated_emits: Vec::new() };
        }};
    }

    if authorization.node_id != this_node_id {
        fail!(
            DCodeInvocationOutcome::StaleAuthority,
            "authorization_wrong_node",
            format!("authorization targets node '{}', not this node '{this_node_id}'", authorization.node_id)
        );
    }

    if artifact.application_id != authorization.application_id {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_application_mismatch",
            format!(
                "artifact application_id '{}' does not match authorization application_id '{}'",
                artifact.application_id, authorization.application_id
            )
        );
    }
    if artifact.dserv_id != authorization.service_id {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_service_mismatch",
            format!(
                "artifact dserv_id '{}' does not match authorization service_id '{}'",
                artifact.dserv_id, authorization.service_id
            )
        );
    }
    if artifact.artifact_id != authorization.dserv_artifact_id {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_id_mismatch",
            format!(
                "artifact artifact_id '{}' does not match authorization dserv_artifact_id '{}'",
                artifact.artifact_id, authorization.dserv_artifact_id
            )
        );
    }

    let artifact_abi = format!("{}/{}", artifact.abi.name, artifact.abi.version);
    if artifact_abi != authorization.dnode_abi {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_abi_mismatch",
            format!("artifact ABI '{artifact_abi}' does not match authorization ABI '{}'", authorization.dnode_abi)
        );
    }
    if artifact.dcode.sha256 != authorization.dserv_artifact_digest {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_digest_mismatch",
            format!(
                "artifact module digest '{}' does not match authorization digest '{}'",
                artifact.dcode.sha256, authorization.dserv_artifact_digest
            )
        );
    }

    // R2: the WHOLE-descriptor digest binding, separate from the module
    // content digest checked just above.
    let descriptor_digest = match recompute_dserv_artifact_digest(artifact) {
        Ok(digest) => digest,
        Err(error) => fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_descriptor_digest_unrecomputable",
            error
        ),
    };
    if descriptor_digest != authorization.dserv_artifact_descriptor_digest {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_descriptor_digest_mismatch",
            format!(
                "artifact descriptor digest '{descriptor_digest}' does not match authorization descriptor digest '{}' -- a field in limits/ports/safety/abi/etc. changed since authorization was issued",
                authorization.dserv_artifact_descriptor_digest
            )
        );
    }

    // F4: defense-in-depth -- require the fetched/offline artifact to still
    // validate structurally against the FULL mirrored contract BEFORE any of
    // its limits/ports/safety are used for anything below. DServer's own
    // registry already validates artifacts at declaration/reload time; this
    // does not replace that, it makes the mirrored validator load-bearing on
    // the REAL execution side too, rather than trusting a fetched/offline
    // descriptor is structurally valid on faith. Never duplicates the rules
    // manually -- it calls the same `validate()` dserver's own registry uses.
    let artifact_findings = artifact.validate();
    if !artifact_findings.is_empty() {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "artifact_invalid",
            format!("fetched/offline D-Serv artifact does not validate structurally: {artifact_findings:?}")
        );
    }

    // R4C: the input port must be one the artifact actually declares.
    let declared_input_ports: Vec<String> = artifact.ports.inputs.iter().map(|port| port.port.clone()).collect();
    if let Err(error) = envelope::validate_input_port(&declared_input_ports, input_port) {
        fail!(DCodeInvocationOutcome::ValidationFailed, "undeclared_input_port", error.to_string());
    }

    let module_bytes = match module_store.load(&authorization.dserv_artifact_digest) {
        Ok(bytes) => bytes,
        Err(error) => fail!(DCodeInvocationOutcome::ValidationFailed, "module_load_failed", error),
    };
    // F3: record the ACTUAL loaded module length directly, before the R3
    // check below can early-return -- so even a size-mismatch failure's
    // evidence honestly reflects what was really loaded.
    evidence.module_size_bytes = Some(module_bytes.len() as u64);

    // R3: declared size_bytes must match the actual loaded module length.
    if module_bytes.len() as u64 != artifact.dcode.size_bytes {
        fail!(
            DCodeInvocationOutcome::ValidationFailed,
            "module_size_mismatch",
            format!(
                "artifact declares dcode.size_bytes={}, but the loaded module is {} bytes",
                artifact.dcode.size_bytes,
                module_bytes.len()
            )
        );
    }

    let limits = InvocationLimits {
        fuel_per_invocation: artifact.limits.fuel_per_invocation,
        timeout_ms: artifact.limits.timeout_ms,
        max_memory_pages: artifact.limits.max_memory_pages,
        max_message_bytes: artifact.limits.max_message_bytes,
    };

    // R4A: host constructs the input envelope; message_id/correlation_id are
    // ALWAYS host-generated, never derived from the guest. Phase 112:
    // `correlation_override` (set only by `invoke_dcode_with_correlation`)
    // propagates one D-Call chain's correlation_id instead of minting a new
    // one; `identity_override` (I2A, set only by
    // `invoke_dcode_with_delivery_identity`) supplies BOTH host-owned
    // identities so a D-Call delivery grant's own bound message_id/
    // correlation_id are the exact values the guest actually receives.
    let (envelope_bytes, message_id, correlation_id, received_at_utc) = match (identity_override, correlation_override) {
        (Some((message_id, correlation_id)), _) => envelope::build_input_envelope_with_identity(input_port, input, message_id, correlation_id),
        (None, Some(correlation_id)) => envelope::build_input_envelope_with_correlation(input_port, input, correlation_id),
        (None, None) => envelope::build_input_envelope(input_port, input),
    };
    evidence.message_id = message_id;
    evidence.correlation_id = correlation_id;
    // F3A: identity of the EXACT bytes/timestamp the guest actually
    // received -- deliberately distinct from `input_sha256`/`input_size`
    // above, which digest the raw caller payload before envelope wrapping.
    evidence.input_envelope_sha256 = sha256_hex(&envelope_bytes);
    evidence.input_envelope_size = envelope_bytes.len();
    evidence.received_at_utc = received_at_utc;

    let raw_output = match executor.execute(&authorization.dserv_artifact_digest, &limits, &envelope_bytes) {
        ExecutionOutcome::Success(output) => output,
        ExecutionOutcome::ValidationFailed(message) => {
            evidence.outcome = DCodeInvocationOutcome::ValidationFailed;
            evidence.error_code = Some("validation_failed".to_string());
            evidence.error_message = Some(message);
            evidence.duration_ms = started.elapsed().as_millis() as u64;
            return DCodeInvocationResult { evidence, validated_emits: Vec::new() };
        }
        ExecutionOutcome::ResourceLimit(message) => {
            evidence.outcome = DCodeInvocationOutcome::ResourceLimit;
            evidence.error_code = Some("resource_limit".to_string());
            evidence.error_message = Some(message);
            evidence.duration_ms = started.elapsed().as_millis() as u64;
            return DCodeInvocationResult { evidence, validated_emits: Vec::new() };
        }
        ExecutionOutcome::Trap(message) => {
            evidence.outcome = DCodeInvocationOutcome::Trap;
            evidence.error_code = Some("trap".to_string());
            evidence.error_message = Some(message);
            evidence.duration_ms = started.elapsed().as_millis() as u64;
            return DCodeInvocationResult { evidence, validated_emits: Vec::new() };
        }
    };

    // H1: the CANONICAL "no output" (exact packed `i64` 0) must never reach
    // JSON-envelope parsing -- it is an infrastructure Success with no output
    // envelope, no status, no reason, no emits. Every evidence field below
    // this point already defaults to `None`/`false`/empty from the initial
    // `evidence` construction above, so simply returning here (after marking
    // the outcome and duration) is sufficient to produce exactly that
    // evidence shape -- it does not "pretend" any output payload existed.
    let raw_output = match raw_output {
        ExecutionOutput::NoOutput => {
            evidence.outcome = DCodeInvocationOutcome::Success;
            evidence.duration_ms = started.elapsed().as_millis() as u64;
            return DCodeInvocationResult { evidence, validated_emits: Vec::new() };
        }
        ExecutionOutput::Bytes(bytes) => bytes,
    };

    evidence.output_sha256 = Some(sha256_hex(&raw_output));
    evidence.output_size = Some(raw_output.len());

    // R4B/R4C: parse and fail-closed-validate the output envelope.
    let mut validated_emits: Vec<ValidatedDCodeEmit> = Vec::new();
    let declared_output_ports: Vec<String> = artifact.ports.outputs.iter().map(|port| port.port.clone()).collect();
    match envelope::parse_and_validate_output(&raw_output, &declared_output_ports, artifact.limits.max_emits_per_invocation)
    {
        Ok(validated) => {
            evidence.output_status = Some(validated.status.clone());
            // G3A: never persist the guest-controlled `reason` string
            // verbatim -- only whether it was present, and its digest/size.
            if let Some(reason) = &validated.reason {
                evidence.output_reason_present = true;
                evidence.output_reason_sha256 = Some(sha256_hex(reason.as_bytes()));
                evidence.output_reason_size = Some(reason.len());
            }
            evidence.emits = validated
                .emits
                .iter()
                .map(|emit| DCodeEmitEvidenceV1 {
                    port: emit.port().to_string(),
                    payload_sha256: sha256_hex(emit.payload()),
                    payload_size: emit.payload().len(),
                })
                .collect();
            // I1A: the REAL decoded emit bytes, transiently, for
            // `invoke_dcode_capturing_emits` callers only -- `invoke_dcode`/
            // `invoke_dcode_with_correlation`/`invoke_dcode_with_delivery_identity`
            // all discard this field via `.evidence`.
            validated_emits = validated.emits;
            // A well-formed envelope -- whether the guest's own business
            // logic reports "ok" or "rejected" -- is a successful
            // INFRASTRUCTURE-level invocation; `output_status` carries the
            // guest's business decision for the caller to act on.
            evidence.outcome = DCodeInvocationOutcome::Success;
        }
        Err(error) => {
            evidence.outcome = DCodeInvocationOutcome::ValidationFailed;
            evidence.error_code = Some(match error {
                EnvelopeError::InvalidUtf8 => "output_invalid_utf8",
                EnvelopeError::InvalidJson(_) => "output_invalid_json",
                EnvelopeError::InvalidStatus(_) => "output_invalid_status",
                EnvelopeError::MalformedBase64(_) => "output_malformed_base64",
                EnvelopeError::TooManyEmits { .. } => "output_too_many_emits",
                EnvelopeError::UndeclaredInputPort(_) => "output_undeclared_input_port",
                EnvelopeError::UndeclaredOutputPort(_) => "output_undeclared_output_port",
            }.to_string());
            // G3B: `error_message` is now a STATIC, safe classification --
            // never the guest-controlled offending value (e.g. an invalid
            // `status` string, or an undeclared emit `port` name) verbatim.
            // `envelope::safe_error_message` also returns that offending
            // value's raw bytes separately, which are digested/sized here,
            // never persisted as-is.
            let (safe_message, offending_value) = envelope::safe_error_message(&error);
            evidence.error_message = Some(safe_message);
            if let Some(value) = offending_value {
                evidence.error_offending_value_sha256 = Some(sha256_hex(&value));
                evidence.error_offending_value_size = Some(value.len());
            }
        }
    }

    evidence.duration_ms = started.elapsed().as_millis() as u64;
    DCodeInvocationResult { evidence, validated_emits }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcode::authorization::CanonicalContractReferenceV1;
    use crate::dcode::dserv_artifact::{
        DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1, DServArtifactLimitsV1,
        DServArtifactPortV1, DServArtifactPortsV1, DServArtifactSafetyV1, CANONICAL_DSERV_ARTIFACT_SCHEMA,
    };
    use crate::dcode::abi::DNODE_ABI_V0_REQUIRED_EXPORTS;

    const ECHO_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (global $next (mut i32) (i32.const 1024))
  (func (export "datum_abi_version") (result i32) (i32.const 0))
  (func (export "datum_alloc") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $next))
    (global.set $next (i32.add (global.get $next) (local.get $len)))
    (local.get $ptr))
  (func (export "datum_dealloc") (param $ptr i32) (param $len i32))
  (func (export "datum_handle") (param $ptr i32) (param $len i32) (result i64)
    (i64.or
      (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
      (i64.extend_i32_u (local.get $len)))))
"#;

    /// The echo module here returns its RAW input verbatim -- since R4 now
    /// wraps input in an envelope before it reaches the guest, "echo" means
    /// "returns the JSON envelope bytes it was given," which is not itself a
    /// valid ADR-0003 output envelope (no `status` field). Tests that need a
    /// genuinely valid OUTPUT envelope use `respond_ok_wat` below instead.
    fn echo_bytes() -> Vec<u8> {
        wat::parse_str(ECHO_WAT).unwrap()
    }

    /// A module that ignores its input and always returns a fixed, valid
    /// ADR-0003 output envelope with the given emits (as a literal JSON
    /// array snippet) baked into the returned bytes via a data segment.
    fn respond_wat(body: &str) -> Vec<u8> {
        let escaped: String = body
            .bytes()
            .map(|b| format!("\\{:02x}", b))
            .collect();
        let module = format!(
            r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 0) "{escaped}")
  (func (export "datum_abi_version") (result i32) (i32.const 0))
  (func (export "datum_alloc") (param $len i32) (result i32) (i32.const 4096))
  (func (export "datum_dealloc") (param $ptr i32) (param $len i32))
  (func (export "datum_handle") (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {}))))
"#,
            body.len()
        );
        wat::parse_str(&module).unwrap()
    }

    fn setup_with_module(module_bytes: Vec<u8>) -> (DCodeModuleCache, DCodeModuleStore, DCodeInvocationAuthorizationV1, CanonicalDServArtifactV1) {
        let digest = sha256_hex(&module_bytes);
        let store = DCodeModuleStore::new(std::env::temp_dir().join(format!("datum-dcode-invoke-{}", uuid::Uuid::new_v4())));
        store.install(&module_bytes, &digest).unwrap();
        let cache = DCodeModuleCache::new().unwrap();

        let artifact = CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: "dserv:sensor-pipeline:decode".to_string(),
            version: "0.1.0".to_string(),
            application_id: "app:sensor-pipeline".to_string(),
            dserv_id: "decode".to_string(),
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{digest}"),
                sha256: digest.clone(),
                content_id: format!("sha256:{digest}"),
                size_bytes: module_bytes.len() as u64,
            },
            abi: DServArtifactAbiV1 {
                name: "datum-dnode".to_string(),
                version: "0".to_string(),
                required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
                allowed_imports: Vec::new(),
            },
            ports: DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 { port: "uplink".to_string(), payload_schema: "s".to_string(), external: true }],
                outputs: vec![DServArtifactPortV1 { port: "reading".to_string(), payload_schema: "s".to_string(), external: false }],
            },
            limits: DServArtifactLimitsV1 {
                fuel_per_invocation: 5_000_000,
                timeout_ms: 200,
                max_memory_pages: 4,
                max_message_bytes: 65536,
                max_emits_per_invocation: 8,
            },
            safety: DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: DServArtifactConstraintsV1::default(),
            qos: serde_json::Value::Null,
            provenance: serde_json::Value::Null,
            probes: serde_json::Value::Null,
            signature_ref: None,
        };
        let descriptor_digest = recompute_dserv_artifact_digest(&artifact).unwrap();

        let authorization = DCodeInvocationAuthorizationV1 {
            authorization_id: "dcode-auth:proj:app:svc:node-1:1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:sensor-pipeline".to_string(),
            service_id: "decode".to_string(),
            node_id: "node-1".to_string(),
            dmap_ref: CanonicalContractReferenceV1 { id: "dmap:1".to_string(), revision: 1, digest: format!("sha256:{}", "1".repeat(64)) },
            dgraph_ref: CanonicalContractReferenceV1 { id: "dgraph:1".to_string(), revision: 1, digest: format!("sha256:{}", "2".repeat(64)) },
            dserv_artifact_id: "dserv:sensor-pipeline:decode".to_string(),
            dserv_artifact_digest: digest,
            dserv_artifact_descriptor_digest: descriptor_digest,
            dnode_abi: "datum-dnode/0".to_string(),
        };
        (cache, store, authorization, artifact)
    }

    fn setup() -> (DCodeModuleCache, DCodeModuleStore, DCodeInvocationAuthorizationV1, CanonicalDServArtifactV1) {
        setup_with_module(respond_wat(r#"{"status":"ok","emits":[]}"#))
    }

    #[test]
    fn authorized_invocation_with_locally_installed_module_succeeds() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
        assert_eq!(evidence.output_status.as_deref(), Some("ok"));
        assert!(evidence.fresh_instance_per_invocation);
        assert!(!evidence.message_id.is_empty());
        assert!(!evidence.correlation_id.is_empty());
        assert_eq!(evidence.input_port, "uplink");
    }

    #[test]
    fn emitted_payload_is_recorded_as_digest_and_size_only() {
        let payload_b64 = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(b"emitted!")
        };
        let (cache, store, authorization, artifact) =
            setup_with_module(respond_wat(&format!(r#"{{"status":"ok","emits":[{{"port":"reading","payload_b64":"{payload_b64}"}}]}}"#)));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
        assert_eq!(evidence.emits.len(), 1);
        assert_eq!(evidence.emits[0].port, "reading");
        assert_eq!(evidence.emits[0].payload_size, 8);
        assert_eq!(evidence.emits[0].payload_sha256, sha256_hex(b"emitted!"));
    }

    #[test]
    fn rejected_status_is_still_an_infrastructure_success() {
        let (cache, store, authorization, artifact) =
            setup_with_module(respond_wat(r#"{"status":"rejected","reason":"business rule declined"}"#));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
        assert_eq!(evidence.output_status.as_deref(), Some("rejected"));
        // G3A: the reason TEXT is never persisted -- only its presence and
        // digest/size.
        assert!(evidence.output_reason_present);
        assert_eq!(evidence.output_reason_sha256, Some(sha256_hex(b"business rule declined")));
        assert_eq!(evidence.output_reason_size, Some("business rule declined".len()));
    }

    // -- H1: canonical "no output" (exact packed `i64` 0) --------------------

    const NO_OUTPUT_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func (export "datum_abi_version") (result i32) (i32.const 0))
  (func (export "datum_alloc") (param $len i32) (result i32) (i32.const 0))
  (func (export "datum_dealloc") (param $ptr i32) (param $len i32))
  (func (export "datum_handle") (param $ptr i32) (param $len i32) (result i64) (i64.const 0)))
"#;

    fn no_output_bytes() -> Vec<u8> {
        wat::parse_str(NO_OUTPUT_WAT).unwrap()
    }

    /// H1 regressions 1/2/3/4/5: a module whose `datum_handle` returns the
    /// EXACT packed value `0` succeeds as an infrastructure-level `Success`,
    /// is NEVER fed into JSON-envelope parsing (proven indirectly: if it
    /// were, this would fail as `output_invalid_json` since an empty byte
    /// string is not valid JSON), records no `output_status`, no emits, and
    /// no output digest/size -- evidence does not pretend an output payload
    /// existed.
    #[test]
    fn exact_no_output_succeeds_without_pretending_a_payload_existed() {
        let (cache, store, authorization, artifact) = setup_with_module(no_output_bytes());
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
        assert_eq!(evidence.error_code, None);
        assert_eq!(evidence.output_status, None);
        assert_eq!(evidence.output_sha256, None);
        assert_eq!(evidence.output_size, None);
        assert!(evidence.emits.is_empty());
        assert!(!evidence.output_reason_present);
        assert_eq!(evidence.output_reason_sha256, None);
        assert_eq!(evidence.output_reason_size, None);
    }

    /// H1 regression 7: ordinary (non-no-output) JSON output continues to
    /// work exactly as before this fix.
    #[test]
    fn ordinary_json_output_is_unaffected_by_the_no_output_fix() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
        assert_eq!(evidence.output_status.as_deref(), Some("ok"));
    }

    /// H1 regression 8: malformed NON-empty JSON output still fails exactly
    /// as before -- unaffected by the no-output fix, which applies only to
    /// the exact packed value `0`.
    #[test]
    fn malformed_non_empty_json_output_still_fails() {
        let (cache, store, authorization, artifact) = setup_with_module(echo_bytes());
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("output_invalid_json"));
    }

    #[test]
    fn malformed_output_envelope_is_rejected() {
        let (cache, store, authorization, artifact) = setup_with_module(echo_bytes());
        let executor = InProcessExecutor { cache: &cache, store: &store };
        // Echo module returns whatever bytes it was given -- since input is
        // now envelope-wrapped JSON with no top-level "status" field, this
        // is NOT a valid output envelope.
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("output_invalid_json"));
    }

    #[test]
    fn wrong_node_is_rejected_as_stale_authority() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-2", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::StaleAuthority);
    }

    #[test]
    fn artifact_digest_mismatch_is_rejected() {
        let (cache, store, authorization, mut artifact) = setup();
        artifact.dcode.sha256 = "0".repeat(64);
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("artifact_digest_mismatch"));
    }

    /// R2 regression: changing only a descriptor field (limits) while the
    /// module digest stays identical must be caught by the REAL invocation
    /// path, not merely by the server-side authorization check.
    #[test]
    fn changing_only_limits_is_caught_by_the_real_invocation_path() {
        let (cache, store, authorization, mut artifact) = setup();
        artifact.limits.fuel_per_invocation *= 2; // module bytes/sha256 unchanged
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("artifact_descriptor_digest_mismatch"));
    }

    /// R3 regression: declared size_bytes not matching the real module.
    #[test]
    fn wrong_declared_size_bytes_is_rejected() {
        let (cache, store, mut authorization, mut artifact) = setup();
        artifact.dcode.size_bytes += 1;
        // size_bytes is itself part of the descriptor -- re-derive the
        // authorization's descriptor digest to match, so this test isolates
        // the R3 byte-length check specifically, not the (already separately
        // tested) R2 descriptor-digest binding.
        authorization.dserv_artifact_descriptor_digest = recompute_dserv_artifact_digest(&artifact).unwrap();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("module_size_mismatch"));
    }

    /// R4C regression: an input port the artifact never declared.
    #[test]
    fn undeclared_input_port_is_rejected() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "not-a-real-port", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("undeclared_input_port"));
    }

    #[test]
    fn undeclared_emit_port_is_rejected() {
        let (cache, store, authorization, artifact) =
            setup_with_module(respond_wat(r#"{"status":"ok","emits":[{"port":"not-declared","payload_b64":""}]}"#));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("output_undeclared_output_port"));
    }

    #[test]
    fn too_many_emits_is_rejected() {
        let (cache, store, authorization, artifact) = setup_with_module(respond_wat(
            r#"{"status":"ok","emits":[{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""},{"port":"reading","payload_b64":""}]}"#,
        ));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("output_too_many_emits"));
    }

    #[test]
    fn invocation_creates_no_dmap_or_placement_mutation() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let before = format!("{authorization:?}");
        let _ = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(format!("{authorization:?}"), before, "authorization must remain byte-for-byte unchanged");
    }

    #[test]
    fn host_generated_message_and_correlation_id_are_never_equal_across_two_calls() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let e1 = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        let e2 = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_ne!(e1.message_id, e2.message_id);
        assert_ne!(e1.correlation_id, e2.correlation_id);
    }

    // -- F3/F3A: execution evidence identity completeness -------------------

    #[test]
    fn evidence_carries_exact_authorization_dmap_and_dgraph_digests() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.dmap_digest, authorization.dmap_ref.digest);
        assert_eq!(evidence.dgraph_digest, authorization.dgraph_ref.digest);
        assert!(!evidence.dmap_digest.is_empty());
        assert!(!evidence.dgraph_digest.is_empty());
    }

    #[test]
    fn evidence_carries_exact_artifact_descriptor_digest_separate_from_module_digest() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.dserv_artifact_descriptor_digest, authorization.dserv_artifact_descriptor_digest);
        assert_eq!(evidence.dserv_artifact_digest, authorization.dserv_artifact_digest);
        assert_ne!(
            evidence.dserv_artifact_descriptor_digest, evidence.dserv_artifact_digest,
            "descriptor digest and module digest must remain two separate identities in evidence too"
        );
    }

    #[test]
    fn evidence_module_size_equals_actual_loaded_module_length() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.module_size_bytes, Some(artifact.dcode.size_bytes));
    }

    #[test]
    fn evidence_module_size_is_absent_when_invocation_fails_before_module_load() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        // Wrong node fails BEFORE the module is ever loaded.
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-2", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::StaleAuthority);
        assert_eq!(evidence.module_size_bytes, None);
    }

    /// Captures the exact bytes the top-level `invoke_dcode` function hands
    /// to the executor -- i.e. the exact serialized ADR-0003 input envelope,
    /// not the raw caller payload -- so evidence claims about that envelope
    /// can be checked against ground truth rather than merely re-asserting
    /// what `invoke_dcode` itself computed.
    struct CapturingExecutor<'a> {
        inner: InProcessExecutor<'a>,
        captured_input: std::cell::RefCell<Option<Vec<u8>>>,
    }

    impl DCodeExecutor for CapturingExecutor<'_> {
        fn execute(&self, digest: &str, limits: &InvocationLimits, input: &[u8]) -> ExecutionOutcome {
            *self.captured_input.borrow_mut() = Some(input.to_vec());
            self.inner.execute(digest, limits, input)
        }
    }

    #[test]
    fn evidence_envelope_digest_and_size_match_the_exact_bytes_sent_to_the_executor() {
        let (cache, store, authorization, artifact) = setup();
        let executor =
            CapturingExecutor { inner: InProcessExecutor { cache: &cache, store: &store }, captured_input: std::cell::RefCell::new(None) };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        let captured = executor.captured_input.borrow().clone().expect("executor must have been called");

        assert_eq!(evidence.input_envelope_sha256, sha256_hex(&captured));
        assert_eq!(evidence.input_envelope_size, captured.len());

        // received_at_utc in evidence must be the SAME timestamp embedded in
        // the envelope actually sent to the guest, not independently
        // regenerated afterward.
        let envelope_value: serde_json::Value = serde_json::from_slice(&captured).unwrap();
        assert_eq!(envelope_value["received_at_utc"], evidence.received_at_utc);
        assert!(!evidence.received_at_utc.is_empty());
    }

    #[test]
    fn raw_payload_digest_remains_distinct_from_envelope_digest() {
        let (cache, store, authorization, artifact) = setup();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.input_sha256, sha256_hex(b"hello"));
        assert_ne!(
            evidence.input_sha256, evidence.input_envelope_sha256,
            "raw payload digest and envelope digest must remain two separate identities"
        );
        assert!(
            evidence.input_envelope_size > evidence.input_size,
            "the JSON envelope wrapping the raw payload must be larger than the raw payload alone"
        );
    }

    /// F4 regression: a structurally invalid artifact (here, a zero limit
    /// that `validate()` rejects) must fail closed at the new local
    /// `artifact.validate()` gate, proving the mirrored validator is
    /// load-bearing on the real execution side, not merely present.
    #[test]
    fn structurally_invalid_artifact_is_rejected_before_use() {
        let (cache, store, mut authorization, mut artifact) = setup();
        artifact.limits.max_emits_per_invocation = 0; // validate() rejects this
        authorization.dserv_artifact_descriptor_digest = recompute_dserv_artifact_digest(&artifact).unwrap();
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"hello");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("artifact_invalid"));
    }

    #[test]
    fn evidence_json_never_contains_the_full_raw_payload_or_full_emitted_payload() {
        let payload_marker = "UNIQUE_RAW_PAYLOAD_MARKER_never_persisted";
        let emit_marker = "UNIQUE_EMITTED_PAYLOAD_MARKER_never_persisted";
        let payload_b64 = {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(emit_marker.as_bytes())
        };
        let (cache, store, authorization, artifact) =
            setup_with_module(respond_wat(&format!(r#"{{"status":"ok","emits":[{{"port":"reading","payload_b64":"{payload_b64}"}}]}}"#)));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", payload_marker.as_bytes());
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);

        let evidence_json = serde_json::to_string(&evidence).unwrap();
        assert!(!evidence_json.contains(payload_marker), "the full raw caller payload must never appear in evidence JSON");
        assert!(!evidence_json.contains(emit_marker), "the full emitted payload must never appear in evidence JSON");
        assert!(!evidence_json.contains(&payload_b64), "the base64 emitted payload must never appear in evidence JSON");
    }

    // -- G3C: adversarial evidence-exfiltration tests ------------------------
    // A malicious/buggy guest module tries to smuggle a unique marker into
    // persisted evidence through every guest-controlled output channel. None
    // must succeed -- evidence may record that something guest-controlled
    // existed (a digest/size/boolean), never the value itself.

    const EXFIL_MARKER: &str = "SUPER_SECRET_DCODE_PAYLOAD_MARKER_should_never_appear_in_evidence_json";

    /// G3C-1/7/8/9/10: exfiltration attempt via the output envelope `reason`
    /// field.
    #[test]
    fn evidence_never_leaks_a_marker_via_output_reason() {
        let (cache, store, authorization, artifact) =
            setup_with_module(respond_wat(&format!(r#"{{"status":"rejected","reason":"{EXFIL_MARKER}"}}"#)));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
        assert!(evidence.output_reason_present);
        // 7/8: digest/size correctly identify the REAL reason bytes, without
        // the marker ever appearing in the JSON serialization (9 below).
        assert!(evidence.output_reason_sha256.is_some());
        assert!(evidence.output_reason_size.unwrap() > 0);

        let evidence_json = serde_json::to_string(&evidence).unwrap();
        assert!(!evidence_json.contains(EXFIL_MARKER), "the reason marker must never appear verbatim in evidence JSON");
    }

    /// G3C-2/10: exfiltration attempt via an invalid `status` value.
    #[test]
    fn evidence_never_leaks_a_marker_via_invalid_status() {
        let (cache, store, authorization, artifact) =
            setup_with_module(respond_wat(&format!(r#"{{"status":"{EXFIL_MARKER}"}}"#)));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        // 10: error_code is a deterministic, non-content-bearing classifier.
        assert_eq!(evidence.error_code.as_deref(), Some("output_invalid_status"));
        assert_eq!(evidence.error_offending_value_sha256, Some(sha256_hex(EXFIL_MARKER.as_bytes())));
        assert_eq!(evidence.error_offending_value_size, Some(EXFIL_MARKER.len()));

        let evidence_json = serde_json::to_string(&evidence).unwrap();
        assert!(!evidence_json.contains(EXFIL_MARKER), "the invalid status marker must never appear verbatim in evidence JSON");
    }

    /// G3C-3/10: exfiltration attempt via an undeclared output port name.
    #[test]
    fn evidence_never_leaks_a_marker_via_undeclared_output_port() {
        let (cache, store, authorization, artifact) = setup_with_module(respond_wat(&format!(
            r#"{{"status":"ok","emits":[{{"port":"{EXFIL_MARKER}","payload_b64":""}}]}}"#
        )));
        let executor = InProcessExecutor { cache: &cache, store: &store };
        let evidence = invoke_dcode(&executor, &store, &authorization, &artifact, "node-1", "uplink", b"x");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::ValidationFailed);
        assert_eq!(evidence.error_code.as_deref(), Some("output_undeclared_output_port"));
        assert_eq!(evidence.error_offending_value_sha256, Some(sha256_hex(EXFIL_MARKER.as_bytes())));

        let evidence_json = serde_json::to_string(&evidence).unwrap();
        assert!(!evidence_json.contains(EXFIL_MARKER), "the undeclared-port marker must never appear verbatim in evidence JSON");
    }
}
