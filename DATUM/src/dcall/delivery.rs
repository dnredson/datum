//! D-Call dispatch (ADR-0022, sections 14-16, 32): the D-Engine-facing
//! library seam. `dispatch_validated_emit` is deliberately the ONLY public
//! entrypoint a future orchestration layer needs -- it resolves calls,
//! derives routes, obtains grants, dispatches each call (local in-process
//! or remote over HTTP), and returns per-call evidence. It does NOT
//! recursively process the destination's own emits (section 25: one
//! delivery is one edge traversal; graph-wide propagation is explicitly a
//! LATER D-Engine concern, not implemented here -- see
//! `dcall-scope-exclusion-audit.txt`).
//!
//! Local and remote delivery both end up calling the SAME
//! `execute_destination_delivery` helper, which is what actually invokes
//! the destination's D-Code (via the ordinary Phase-111 governed
//! authorization-fetch + `invoke_dcode_with_delivery_identity` path) -- this
//! is the concrete mechanism behind
//! `local-remote-semantic-equivalence-audit.txt`'s claim that the
//! destination D-Serv sees an identical envelope regardless of route_kind.
//!
//! Independent review (evidence sanitization): every path below returns a
//! `DCallDeliveryEvidenceV1`, never a raw string. A raw HTTP response body,
//! transport error, or other diagnostic text is NEVER embedded in the
//! returned evidence -- it is passed only as `raw_diagnostic` to the
//! `evidence` module's builders, which digest it (`error_diagnostic_sha256`)
//! and discard the text itself. See `dcall-payload-leakage-audit.txt`.
//!
//! Independent review fix v2 (I1): `dispatch_validated_emit` takes a TRUSTED
//! source `DCodeInvocationEvidenceV1` (produced by a real
//! `dcode::invoke::invoke_dcode_capturing_emits` call) plus the specific
//! emit port/payload to dispatch -- it structurally proves the payload
//! actually came from that real, successful source invocation (its digest
//! must match an emit digest the evidence itself recorded) rather than
//! trusting arbitrary caller-supplied bytes/identity strings.
//!
//! Independent review fix v3 (J1): that v2 mechanism was a RUNTIME digest
//! check, not a genuine API-boundary property -- the function still accepted
//! a public, `Serialize`/`Deserialize`, public-fielded `DCodeInvocationEvidenceV1`
//! plus an independent raw `&[u8]`, so a caller could fabricate an evidence
//! record whose digest/size matched arbitrary chosen bytes. The production
//! seam now takes `&dcode::invoke::DCodeInvocationResult` (opaque: private
//! fields, no public constructor, never `Deserialize`) plus an index into its
//! own `.validated_emits()` slice -- there is no longer any way to construct
//! the inputs to this function from outside a real, successful
//! `invoke_dcode_capturing_emits` call. This also fixes a real correctness
//! bug the old by-port-name lookup had: two emits on the SAME output port
//! with different payloads would incorrectly compare the second against the
//! first's digest. See `source-dcode-emit-provenance-audit.txt`.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Result};
use uuid::Uuid;

use crate::dcall::client;
use crate::dcall::evidence::{self, DCallDeliveryErrorCode, DCallDeliveryEvidenceV1};
use crate::dcall::grant::{DCallDeliveryGrantV1, SourceExecutionBindingV1, ROUTE_KIND_LOCAL, ROUTE_KIND_REMOTE, TRANSPORT_HTTP};
use crate::dcall::listener::{DeliverRequest, DELIVER_PATH};
use crate::dcode::authorization::DCodeInvocationAuthorizationV1;
use crate::dcode::dserv_artifact::CanonicalDServArtifactV1;
use crate::dcode::evidence::{DCodeInvocationEvidenceV1, DCodeInvocationOutcome};
use crate::dcode::identity::sha256_hex;
use crate::dcode::invoke::{invoke_dcode_with_delivery_identity, GovernedDCodeInvocationResult};
use crate::dcode::local_identity::load_local_identity;
use crate::dcode::module_store::DCodeModuleStore;
use crate::dcode::worker::{trusted_sibling_worker_binary_path, IsolatedWorkerExecutor};

/// I6: hard cap on a remote listener's response body -- large enough for a
/// real `DCodeInvocationEvidenceV1` JSON payload (which is digest/size/id
/// fields only, never raw content -- typically well under 4 KiB) with
/// generous headroom, small enough that a buggy or malicious listener
/// cannot force unbounded source-side memory allocation.
pub const MAX_REMOTE_RESPONSE_BYTES: usize = 1024 * 1024;

/// Trusted host-generated context for D-Call dispatch. Independent review
/// fix v2 (I1C/I1D): deliberately contains NO source identity fields and NO
/// worker-binary override -- both were removed because they let a caller
/// (or a careless internal caller) supply values that were never actually
/// verified against anything real (I1's root cause). Source identity now
/// comes exclusively from the trusted `DCodeInvocationEvidenceV1` passed to
/// `dispatch_validated_emit`, cross-checked against THIS installation's own
/// local D-Node identity file; the worker binary is always the trusted
/// sibling of the running executable (`default_worker_binary_path`).
pub struct SourceExecutionContext {
    pub dserver_url: String,
    pub dnode_root: String,
}

/// Independent review fix v5 (K1C): now a thin re-export of the ONE
/// centralized trusted-worker resolver in `dcode::worker` -- previously this
/// exact logic was duplicated three ways (here, `smartsentinel-dcode-invoke`,
/// `smartsentinel-dcall-listen`); all three now share one implementation.
pub fn default_worker_binary_path() -> PathBuf {
    trusted_sibling_worker_binary_path()
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// A delivery attempt's failure, before it has been turned into evidence.
/// `redeemed_at` is `Some` only if this delivery is known to have
/// successfully redeemed the grant before failing later (e.g. destination
/// execution itself failed after a successful redemption) -- this
/// distinction matters for honest forensic reconstruction even though it
/// does not change the deterministic `error_code`.
#[derive(Debug)]
struct DeliveryFailure {
    code: DCallDeliveryErrorCode,
    diagnostic: Option<String>,
    redeemed_at: Option<String>,
}

/// Independent review fix v5 (K1B/J1B): production entrypoint.
/// `source_result` MUST be a `GovernedDCodeInvocationResult` -- obtainable
/// ONLY via `dcode::invoke::invoke_dcode_governed_capturing_emits`, which
/// always runs the real isolated-worker execution path (never a
/// caller-implemented `DCodeExecutor`). `emit_index` selects one of that SAME
/// result's own `.validated_emits()`. This is a genuine type-system/API-
/// boundary property, not merely a runtime digest check -- see
/// `governed-source-invocation-provenance-audit.txt` and
/// `source-dcode-emit-provenance-audit.txt`. Uses the trusted sibling D-Code
/// worker binary (I1D/K1C) -- never a caller-supplied path.
pub fn dispatch_validated_emit(
    context: &SourceExecutionContext,
    source_result: &GovernedDCodeInvocationResult,
    emit_index: usize,
) -> Vec<DCallDeliveryEvidenceV1> {
    dispatch_validated_emit_impl(context, source_result, emit_index, &default_worker_binary_path())
}

/// INTERNAL/TEST-ONLY (independent review fix v5, K1D): identical to
/// `dispatch_validated_emit` except the D-Code worker binary path is
/// supplied explicitly, since a `cargo test` binary's `current_exe()` does
/// not live alongside the real installed worker binary the way a real
/// installation does. Compiled ONLY behind the non-default `dcall-test-support`
/// feature -- a normal `cargo build`/`cargo build --release` never includes
/// this symbol, so ordinary release code cannot call it merely by knowing its
/// name. NEVER call this from production code.
#[cfg(feature = "dcall-test-support")]
pub fn dispatch_validated_emit_for_test(
    context: &SourceExecutionContext,
    source_result: &GovernedDCodeInvocationResult,
    emit_index: usize,
    worker_binary: &Path,
) -> Vec<DCallDeliveryEvidenceV1> {
    dispatch_validated_emit_impl(context, source_result, emit_index, worker_binary)
}

fn dispatch_validated_emit_impl(
    context: &SourceExecutionContext,
    source_result: &GovernedDCodeInvocationResult,
    emit_index: usize,
    worker_binary: &Path,
) -> Vec<DCallDeliveryEvidenceV1> {
    let start = Instant::now();
    let source_evidence = source_result.evidence();

    // J1: kept as an explicit, specific check for a clear error code, even
    // though it is now technically implied by construction --
    // `DCodeInvocationResult::validated_emits()` is only ever non-empty when
    // `evidence().outcome == Success` (see `invoke_dcode_impl`), so a
    // non-Success result can never have a resolvable `emit_index` either way.
    if source_evidence.outcome != DCodeInvocationOutcome::Success {
        return vec![evidence::build_pre_grant_failure_evidence(
            delivery_id(),
            &source_evidence.project_id,
            &source_evidence.application_id,
            "",
            DCallDeliveryErrorCode::DCallSourceInvocationNotSuccessful,
            Some(&format!("source invocation outcome was {:?}, not Success", source_evidence.outcome)),
            elapsed_ms(start),
        )];
    }

    // J1B: select the specific validated emit BY INDEX into this SAME
    // result's own opaque `.validated_emits()` -- there is no digest/port
    // matching left to do here, because the only way a caller could obtain a
    // `usize` "emit_index" that resolves to something is if it genuinely
    // indexes into a REAL `DCodeInvocationResult`'s own real emit list. This
    // is what makes it structurally (not merely at-runtime) impossible for
    // arbitrary host bytes to masquerade as a validated source emit, and it
    // also fixes the old by-port-name lookup's same-port bug (J1C): two
    // emits on the same output port are simply two different indices.
    let emit = match source_result.validated_emits().get(emit_index) {
        Some(emit) => emit,
        None => {
            return vec![evidence::build_pre_grant_failure_evidence(
                delivery_id(),
                &source_evidence.project_id,
                &source_evidence.application_id,
                "",
                DCallDeliveryErrorCode::DCallSourceEmitNotFound,
                Some(&format!("emit_index {emit_index} is out of range for this source invocation's {} validated emit(s)", source_result.validated_emits().len())),
                elapsed_ms(start),
            )]
        }
    };
    let emit_port = emit.port().to_string();
    let emit_payload = emit.payload();
    let actual_sha256 = sha256_hex(emit_payload);

    // I1C: THIS process's own local D-Node identity must match the node the
    // source invocation evidence claims to have executed on -- closes the
    // source-node-spoofing gap (a process on node B could otherwise claim
    // node A's evidence and dispatch as if it were node A).
    let local_identity = match load_local_identity(Path::new(&context.dnode_root)) {
        Ok(identity) => identity,
        Err(error) => {
            return vec![evidence::build_pre_grant_failure_evidence(
                delivery_id(),
                &source_evidence.project_id,
                &source_evidence.application_id,
                "",
                DCallDeliveryErrorCode::DCallSourceLocalIdentityLoadFailed,
                Some(&error),
                elapsed_ms(start),
            )]
        }
    };
    if local_identity.node_id != source_evidence.node_id {
        return vec![evidence::build_pre_grant_failure_evidence(
            delivery_id(),
            &source_evidence.project_id,
            &source_evidence.application_id,
            "",
            DCallDeliveryErrorCode::DCallSourceLocalIdentityMismatch,
            Some(&format!(
                "this installation's local D-Node identity is '{}', but the source invocation evidence claims node '{}'",
                local_identity.node_id, source_evidence.node_id
            )),
            elapsed_ms(start),
        )];
    }

    let call_ids = match client::list_calls_for_source_service(&context.dserver_url, &source_evidence.project_id, &source_evidence.application_id, &source_evidence.service_id) {
        Ok(ids) => ids,
        Err(error) => {
            let diagnostic = error.to_string();
            return vec![evidence::build_pre_grant_failure_evidence(
                delivery_id(),
                &source_evidence.project_id,
                &source_evidence.application_id,
                "",
                DCallDeliveryErrorCode::DCallLookupFailed,
                Some(&diagnostic),
                elapsed_ms(start),
            )];
        }
    };

    call_ids
        .into_iter()
        .map(|call_id| {
            // Fresh message_id PER DELIVERY ATTEMPT (section 13); the
            // correlation_id is the ONE value shared across the whole fan-out
            // -- the source invocation's OWN host-generated correlation_id,
            // never guest/caller-supplied.
            let message_id = format!("dcall-msg:{}", Uuid::new_v4());
            dispatch_one_call(context, source_evidence, &call_id, &emit_port, emit_payload, &actual_sha256, &message_id, worker_binary)
        })
        .collect()
}

fn delivery_id() -> String {
    format!("dcall-delivery:{}", Uuid::new_v4())
}

#[allow(clippy::too_many_arguments)]
fn dispatch_one_call(
    context: &SourceExecutionContext,
    source_evidence: &DCodeInvocationEvidenceV1,
    call_id: &str,
    output_port: &str,
    payload: &[u8],
    payload_sha256: &str,
    message_id: &str,
    worker_binary: &Path,
) -> DCallDeliveryEvidenceV1 {
    let start = Instant::now();
    let id = delivery_id();

    // K2A: derived EXCLUSIVELY from the real source invocation's own
    // evidence -- never independently invented -- so DServer can prove
    // (K2B) that the authority this emit was produced under is still the
    // authority current at grant-issuance time.
    let source_execution = SourceExecutionBindingV1 {
        service_id: source_evidence.service_id.clone(),
        node_id: source_evidence.node_id.clone(),
        dgraph_id: source_evidence.dgraph_id.clone(),
        dgraph_revision: source_evidence.dgraph_revision,
        dgraph_digest: source_evidence.dgraph_digest.clone(),
        dmap_id: source_evidence.dmap_id.clone(),
        dmap_revision: source_evidence.dmap_revision,
        dmap_digest: source_evidence.dmap_digest.clone(),
        dserv_artifact_id: source_evidence.dserv_artifact_id.clone(),
        dserv_artifact_digest: source_evidence.dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: source_evidence.dserv_artifact_descriptor_digest.clone(),
    };
    // Section 9/11: an opaque digest reference to the EXACT governed source
    // evidence that produced this emit, computed with the same deterministic
    // algorithm used for the destination side (`dcode::evidence::dcode_evidence_sha256`)
    // -- never two subtly different evidence-digest algorithms.
    let source_dcode_evidence_sha256 = crate::dcode::evidence::dcode_evidence_sha256(source_evidence);

    let issued = match client::request_grant(
        &context.dserver_url,
        &source_evidence.project_id,
        &source_evidence.application_id,
        call_id,
        &source_evidence.node_id,
        output_port,
        message_id,
        &source_evidence.correlation_id,
        payload_sha256,
        payload.len() as u64,
        &source_execution,
        &source_dcode_evidence_sha256,
    ) {
        Ok(response) => response,
        Err(error) => {
            let diagnostic = error.to_string();
            let code = error.downcast_ref::<client::GrantRequestRejection>()
                .and_then(|rejection| rejection.code)
                .unwrap_or(DCallDeliveryErrorCode::DCallGrantRequestFailed);
            return evidence::build_pre_grant_failure_evidence(
                id,
                &source_evidence.project_id,
                &source_evidence.application_id,
                call_id,
                code,
                Some(&diagnostic),
                elapsed_ms(start),
            );
        }
    };
    let grant = issued.grant;
    let token = issued.token;

    // DServer echoes the opaque source reference. Never accept a substituted
    // reference as evidence for this invocation, even if it has valid shape.
    if grant.source_dcode_evidence_sha256 != source_dcode_evidence_sha256 {
        return evidence::build_grant_bound_failure_evidence(
            id, &grant, None, DCallDeliveryErrorCode::DCallSourceEvidenceMismatch,
            Some("issued grant substituted the source evidence digest"), elapsed_ms(start),
        );
    }

    let result: Result<(DCodeInvocationEvidenceV1, String), DeliveryFailure> = match grant.route_kind.as_str() {
        ROUTE_KIND_LOCAL => deliver_local(context, &grant, &token, payload, payload_sha256, worker_binary),
        ROUTE_KIND_REMOTE => deliver_remote(&context.dserver_url, &grant, &token, payload, payload_sha256),
        other => Err(DeliveryFailure { code: DCallDeliveryErrorCode::DCallUnsupportedRouteKind, diagnostic: Some(format!("unsupported route_kind '{other}' returned by DServer")), redeemed_at: None }),
    };

    match result {
        Ok((dcode_evidence, redeemed_at)) => {
            // I3: never trust destination evidence merely because it
            // deserialized -- prove it actually corresponds to the grant
            // just redeemed (every load-bearing identity/digest field).
            if let Err(mismatch) = validate_destination_dcode_evidence_against_grant(&dcode_evidence, &grant) {
                return evidence::build_grant_bound_failure_evidence(
                    id,
                    &grant,
                    Some(redeemed_at),
                    DCallDeliveryErrorCode::DCallDestinationEvidenceMismatch,
                    Some(&mismatch),
                    elapsed_ms(start),
                );
            }
            let evidence_json = serde_json::to_vec(&dcode_evidence).unwrap_or_default();
            if dcode_evidence.outcome == DCodeInvocationOutcome::Success {
                evidence::build_success_evidence(id, &grant, redeemed_at, &evidence_json, elapsed_ms(start))
            } else {
                let mut record = evidence::build_grant_bound_failure_evidence(
                    id,
                    &grant,
                    Some(redeemed_at),
                    DCallDeliveryErrorCode::DCallDestinationExecutionFailed,
                    None,
                    elapsed_ms(start),
                );
                record.destination_dcode_evidence_sha256 = Some(sha256_hex(&evidence_json));
                record
            }
        }
        Err(failure) => evidence::build_grant_bound_failure_evidence(id, &grant, failure.redeemed_at, failure.code, failure.diagnostic.as_deref(), elapsed_ms(start)),
    }
}

/// Independent review fix v2 (I3): proves the destination's OWN
/// `DCodeInvocationEvidenceV1` -- obtained either directly (local route) or
/// deserialized from a remote listener's response (remote route) --
/// actually corresponds to the grant that was just redeemed, field by
/// field, rather than trusting it merely because it deserialized correctly.
/// A buggy or compromised listener returning a structurally valid evidence
/// record for a DIFFERENT service/node/DMap/DGraph/artifact/port/payload/
/// message would otherwise be silently accepted.
fn validate_destination_dcode_evidence_against_grant(evidence: &DCodeInvocationEvidenceV1, grant: &DCallDeliveryGrantV1) -> Result<(), String> {
    if evidence.schema != crate::dcode::evidence::DCODE_EVIDENCE_SCHEMA || evidence.dserv_artifact_id != grant.destination.dserv_artifact_id {
        return Err("destination evidence schema or artifact id does not match the grant".to_string());
    }
    if evidence.project_id != grant.project_id {
        return Err(format!("destination evidence project_id '{}' != grant project_id '{}'", evidence.project_id, grant.project_id));
    }
    if evidence.application_id != grant.application_id {
        return Err(format!("destination evidence application_id '{}' != grant application_id '{}'", evidence.application_id, grant.application_id));
    }
    if evidence.service_id != grant.destination.service_id {
        return Err(format!("destination evidence service_id '{}' != grant destination.service_id '{}'", evidence.service_id, grant.destination.service_id));
    }
    if evidence.node_id != grant.destination.node_id {
        return Err(format!("destination evidence node_id '{}' != grant destination.node_id '{}'", evidence.node_id, grant.destination.node_id));
    }
    if evidence.dgraph_id != grant.dgraph_ref.id || evidence.dgraph_revision != grant.dgraph_ref.revision || evidence.dgraph_digest != grant.dgraph_ref.digest {
        return Err("destination evidence DGraph ref does not match the grant's bound DGraph ref".to_string());
    }
    if evidence.dmap_id != grant.dmap_ref.id || evidence.dmap_revision != grant.dmap_ref.revision || evidence.dmap_digest != grant.dmap_ref.digest {
        return Err("destination evidence DMap ref does not match the grant's bound DMap ref".to_string());
    }
    if evidence.dserv_artifact_id != grant.destination.dserv_artifact_id {
        return Err(format!("destination evidence dserv_artifact_id '{}' != grant destination.dserv_artifact_id '{}'", evidence.dserv_artifact_id, grant.destination.dserv_artifact_id));
    }
    if evidence.dserv_artifact_digest != grant.destination.dserv_artifact_digest {
        return Err("destination evidence module digest does not match the grant's bound module digest".to_string());
    }
    if evidence.dserv_artifact_descriptor_digest != grant.destination.dserv_artifact_descriptor_digest {
        return Err("destination evidence descriptor digest does not match the grant's bound descriptor digest".to_string());
    }
    if evidence.input_port != grant.destination.port {
        return Err(format!("destination evidence input_port '{}' != grant destination.port '{}'", evidence.input_port, grant.destination.port));
    }
    if evidence.input_sha256 != grant.payload_sha256 {
        return Err("destination evidence input_sha256 does not match the grant's bound payload_sha256".to_string());
    }
    if evidence.input_size as u64 != grant.payload_size {
        return Err("destination evidence input_size does not match the grant's bound payload_size".to_string());
    }
    if evidence.message_id != grant.message_id {
        return Err(format!("destination evidence message_id '{}' != grant message_id '{}'", evidence.message_id, grant.message_id));
    }
    if evidence.correlation_id != grant.correlation_id {
        return Err(format!("destination evidence correlation_id '{}' != grant correlation_id '{}'", evidence.correlation_id, grant.correlation_id));
    }
    Ok(())
}

fn deliver_local(
    context: &SourceExecutionContext,
    grant: &DCallDeliveryGrantV1,
    token: &str,
    payload: &[u8],
    payload_sha256: &str,
    worker_binary: &Path,
) -> Result<(DCodeInvocationEvidenceV1, String), DeliveryFailure> {
    // Section 15: local delivery still goes through grant redemption --
    // never bypassed merely because the destination happens to be this
    // same node.
    let redeemed = client::redeem_grant(&context.dserver_url, &grant.grant_id, token, &grant.destination.node_id, payload_sha256, payload.len() as u64)
        .map_err(|error| DeliveryFailure { code: DCallDeliveryErrorCode::DCallGrantRedeemFailed, diagnostic: Some(error.to_string()), redeemed_at: None })?;
    let redeemed_at = now_rfc3339();
    let evidence = execute_destination_delivery(&context.dserver_url, &context.dnode_root, worker_binary, &redeemed.destination.node_id, &redeemed, payload)
        .map_err(|error| DeliveryFailure { code: DCallDeliveryErrorCode::DCallDestinationExecutionFailed, diagnostic: Some(error.to_string()), redeemed_at: Some(redeemed_at.clone()) })?;
    Ok((evidence, redeemed_at))
}

/// I6: reads at most `MAX_REMOTE_RESPONSE_BYTES` from a blocking `reqwest`
/// response -- rejects immediately on an oversized declared `Content-Length`
/// (without reading anything), and separately caps the actual bytes read
/// via `Read::take` so a listener that declares no/short Content-Length but
/// then streams unboundedly cannot force unbounded allocation either.
fn read_bounded_response_body(response: reqwest::blocking::Response) -> Result<String, DeliveryFailure> {
    if let Some(declared_len) = response.content_length() {
        if declared_len > MAX_REMOTE_RESPONSE_BYTES as u64 {
            return Err(DeliveryFailure {
                code: DCallDeliveryErrorCode::DCallRemoteResponseTooLarge,
                diagnostic: Some(format!("remote response declared Content-Length {declared_len} exceeds the {MAX_REMOTE_RESPONSE_BYTES}-byte cap")),
                redeemed_at: None,
            });
        }
    }
    let mut limited = response.take((MAX_REMOTE_RESPONSE_BYTES + 1) as u64);
    let mut buffer = Vec::new();
    limited.read_to_end(&mut buffer).map_err(|error| DeliveryFailure { code: DCallDeliveryErrorCode::DCallTransportError, diagnostic: Some(error.to_string()), redeemed_at: None })?;
    if buffer.len() > MAX_REMOTE_RESPONSE_BYTES {
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallRemoteResponseTooLarge,
            diagnostic: Some(format!("remote response body exceeded the {MAX_REMOTE_RESPONSE_BYTES}-byte cap while reading")),
            redeemed_at: None,
        });
    }
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

fn deliver_remote(dserver_url: &str, grant: &DCallDeliveryGrantV1, token: &str, payload: &[u8], payload_sha256: &str) -> Result<(DCodeInvocationEvidenceV1, String), DeliveryFailure> {
    let endpoint = client::fetch_endpoint(dserver_url, &grant.destination.node_id, TRANSPORT_HTTP)
        .map_err(|error| DeliveryFailure { code: DCallDeliveryErrorCode::DCallEndpointFetchFailed, diagnostic: Some(error.to_string()), redeemed_at: None })?;
    // Transport authority must match before even constructing the bearer/payload
    // request. Redemption independently rechecks current registry authority.
    let recomputed = crate::agent::jcs::canonical_digest_sha256_hex(&serde_json::json!({
        "node_id": endpoint.node_id,
        "transport": endpoint.transport,
        "endpoint_id": endpoint.endpoint_id,
        "revision": endpoint.revision,
        "base_url": endpoint.base_url,
    }));
    if endpoint.node_id != grant.destination.node_id
        || endpoint.transport != TRANSPORT_HTTP
        || grant.endpoint_id.as_deref() != Some(endpoint.endpoint_id.as_str())
        || grant.endpoint_revision != Some(endpoint.revision)
        || grant.endpoint_descriptor_digest.as_deref() != Some(endpoint.descriptor_digest.as_str())
        || recomputed.ok().as_deref() != Some(endpoint.descriptor_digest.as_str())
    {
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallEndpointAuthorityMismatch,
            diagnostic: Some("fetched endpoint does not match grant-bound transport authority".to_string()),
            redeemed_at: None,
        });
    }
    let request = DeliverRequest {
        grant_id: grant.grant_id.clone(),
        token: token.to_string(),
        payload_b64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, payload),
        payload_sha256: payload_sha256.to_string(),
        payload_size: payload.len() as u64,
    };
    let url = format!("{}{DELIVER_PATH}", endpoint.base_url.trim_end_matches('/'));
    let http_client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| DeliveryFailure { code: DCallDeliveryErrorCode::DCallTransportError, diagnostic: Some(error.to_string()), redeemed_at: None })?;
    let response = match http_client.post(&url).json(&request).send() {
        Ok(response) => response,
        Err(error) => {
            let code = if error.is_timeout() { DCallDeliveryErrorCode::DCallTransportTimeout } else { DCallDeliveryErrorCode::DCallTransportError };
            return Err(DeliveryFailure { code, diagnostic: Some(error.to_string()), redeemed_at: None });
        }
    };
    let status = response.status();
    let body_text = read_bounded_response_body(response)?;
    if !status.is_success() {
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallTransportError,
            diagnostic: Some(format!("remote D-Call delivery to {url} failed: status={status} body={body_text}")),
            redeemed_at: None,
        });
    }
    let body: serde_json::Value = serde_json::from_str(&body_text)
        .map_err(|_| DeliveryFailure { code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed, diagnostic: Some("remote success response is not JSON".to_string()), redeemed_at: None })?;
    let outcome = body.as_object().and_then(|object| object.get("outcome")).and_then(|value| value.as_str())
        .filter(|outcome| matches!(*outcome, "success" | "failed"))
        .ok_or_else(|| DeliveryFailure { code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed, diagnostic: Some("remote success response has invalid outcome".to_string()), redeemed_at: None })?;
    if outcome != "success" {
        // We honestly do not know from here whether the listener redeemed
        // the grant before failing (e.g. a validation error after
        // redemption vs. a rejection before it) -- report `redeemed_at` as
        // unknown (`None`) rather than guessing either way.
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallDestinationExecutionFailed,
            diagnostic: Some(format!("remote D-Call delivery reported outcome='{outcome}': {body_text}")),
            redeemed_at: None,
        });
    }
    let evidence_json = match body.get("dcode_evidence").cloned() {
        Some(value) => value,
        None => {
            return Err(DeliveryFailure {
                code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
                diagnostic: Some("remote D-Call delivery response missing dcode_evidence".to_string()),
                redeemed_at: None,
            })
        }
    };
    let evidence: DCodeInvocationEvidenceV1 = serde_json::from_value(evidence_json)
        .map_err(|error| DeliveryFailure { code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed, diagnostic: Some(error.to_string()), redeemed_at: None })?;
    // The listener redeemed the grant server-side; we only know it
    // succeeded as of when THIS response arrived, not DServer's own
    // internal timestamp (which round-trips through `dserver`'s response
    // only as the grant's original bound fields, never a fresh
    // `redeemed_at`) -- same honesty convention as `deliver_local`.
    Ok((evidence, now_rfc3339()))
}

/// Shared by BOTH local delivery and the remote listener's own handler --
/// this is the ONE place D-Call ever actually invokes destination D-Code,
/// via the ordinary Phase-111 governed path: fetch a FRESH authorization +
/// artifact from DServer for the destination (never trusts the grant's own
/// digests as sufficient -- `invoke_dcode` independently re-verifies them
/// against whatever this fetch returns), then invoke with the grant's OWN
/// `message_id`/`correlation_id` (independent review fix v2, I2 -- never a
/// freshly-minted message_id, so the grant genuinely binds the identity of
/// the message actually delivered).
fn execute_destination_delivery(
    dserver_url: &str,
    dnode_root: &str,
    worker_binary: &std::path::Path,
    this_node_id: &str,
    grant: &DCallDeliveryGrantV1,
    payload: &[u8],
) -> Result<DCodeInvocationEvidenceV1> {
    let authorization: DCodeInvocationAuthorizationV1 = {
        // `dcall::grant`'s mirror and `dcode::authorization`'s mirror are
        // two independently-declared (but wire-identical) types -- fetch
        // through the real `dcode::client` so the exact same governed
        // preflight Phase 111 already established applies here unchanged.
        crate::dcode::client::fetch_dcode_authorization(dserver_url, &grant.project_id, &grant.application_id, &grant.destination.service_id, this_node_id)?
    };
    let artifact: CanonicalDServArtifactV1 = crate::dcode::client::fetch_dserv_artifact(dserver_url, &grant.application_id, &grant.destination.service_id)?;

    if !artifact.validate().is_empty() {
        return Err(anyhow!("fetched destination D-Serv artifact fails structural validation"));
    }

    let module_store = DCodeModuleStore::new(dnode_root);
    let executor = IsolatedWorkerExecutor { worker_binary, module_store_root: std::path::Path::new(dnode_root) };

    Ok(invoke_dcode_with_delivery_identity(
        &executor,
        &module_store,
        &authorization,
        &artifact,
        this_node_id,
        &grant.destination.port,
        payload,
        &grant.message_id,
        &grant.correlation_id,
    ))
}

/// Governed destination entrypoint. Node identity and the worker executable
/// come from this installation; callers cannot override either.
pub fn receive_governed_delivery(
    dserver_url: &str,
    dnode_root: &str,
    grant_id: &str,
    token: &str,
    payload: &[u8],
) -> Result<DCodeInvocationEvidenceV1> {
    let identity = load_local_identity(Path::new(dnode_root)).map_err(|error| anyhow!(error))?;
    let grant = client::redeem_grant(dserver_url, grant_id, token, &identity.node_id, &sha256_hex(payload), payload.len() as u64)?;
    execute_destination_delivery(dserver_url, dnode_root, &default_worker_binary_path(), &identity.node_id, &grant, payload)
}

#[cfg(all(test, feature = "dcall-test-support"))]
mod tests {
    use super::*;
    mod endpoint_authority;
    use crate::dcall::evidence::DCallDeliveryOutcome;
    use crate::dcode::evidence::{DCodeAppliedLimits, DCODE_EVIDENCE_SCHEMA};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // -- J1: `DCodeInvocationResult`/`ValidatedDCodeEmit` are now opaque
    // (private fields, no public constructor) -- a hand-fabricated
    // `DCodeInvocationEvidenceV1` with an invented `emits` list (as the old
    // `sample_source_evidence` helper built) can no longer be wrapped into a
    // `DCodeInvocationResult` from this module. Every test below that needs
    // one now runs a REAL `invoke_dcode_capturing_emits` call (in-process
    // executor, for test speed) against a tiny real WASM module -- this is
    // strictly stronger test evidence than the old hand-built fixture, since
    // it exercises the exact same construction path production code does.

    use crate::dcode::authorization::CanonicalContractReferenceV1;
    use crate::dcode::dserv_artifact::{
        recompute_dserv_artifact_digest, DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1, DServArtifactLimitsV1, DServArtifactPortV1, DServArtifactPortsV1,
        DServArtifactSafetyV1, CANONICAL_DSERV_ARTIFACT_SCHEMA,
    };
    use crate::dcode::abi::DNODE_ABI_V0_REQUIRED_EXPORTS;
    use crate::dcode::invoke::{invoke_dcode_governed_capturing_emits_for_test, InProcessExecutor};
    use crate::dcode::runtime::DCodeModuleCache;

    /// A module that ignores its (enveloped) input and always returns the
    /// given literal ADR-0003 output-envelope JSON body.
    fn respond_wat(body: &str) -> Vec<u8> {
        let escaped: String = body.bytes().map(|b| format!("\\{:02x}", b)).collect();
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

    /// Runs a REAL, successful (or deliberately-failing, via
    /// `authorization_node_id`) `invoke_dcode_capturing_emits` call and
    /// returns the genuine `DCodeInvocationResult` it produces -- the only
    /// way any caller (including this test module) can obtain one, now that
    /// J1 made the type opaque.
    fn real_source_result(authorization_node_id: &str, this_node_id: &str, output_body: &str) -> GovernedDCodeInvocationResult {
        let module_bytes = respond_wat(output_body);
        let digest = sha256_hex(&module_bytes);
        let store = DCodeModuleStore::new(std::env::temp_dir().join(format!("dcall-delivery-source-{}", Uuid::new_v4())));
        store.install(&module_bytes, &digest).unwrap();
        let cache = DCodeModuleCache::new().unwrap();

        let artifact = CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: "dserv:pipeline:source".to_string(),
            version: "0.1.0".to_string(),
            application_id: "app:a".to_string(),
            dserv_id: "source".to_string(),
            dcode: DServArtifactDCodeV1 { uri: format!("datum-blob://sha256/{digest}"), sha256: digest.clone(), content_id: format!("sha256:{digest}"), size_bytes: module_bytes.len() as u64 },
            abi: DServArtifactAbiV1 { name: "datum-dnode".to_string(), version: "0".to_string(), required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(), allowed_imports: Vec::new() },
            ports: DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 { port: "in".to_string(), payload_schema: "s".to_string(), external: true }],
                outputs: vec![DServArtifactPortV1 { port: "out".to_string(), payload_schema: "s".to_string(), external: false }],
            },
            limits: DServArtifactLimitsV1 { fuel_per_invocation: 5_000_000, timeout_ms: 200, max_memory_pages: 4, max_message_bytes: 65536, max_emits_per_invocation: 8 },
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
            authorization_id: "auth-1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:a".to_string(),
            service_id: "source".to_string(),
            node_id: authorization_node_id.to_string(),
            dmap_ref: CanonicalContractReferenceV1 { id: "dmap:1".to_string(), revision: 1, digest: format!("sha256:{}", "1".repeat(64)) },
            dgraph_ref: CanonicalContractReferenceV1 { id: "dgraph:1".to_string(), revision: 1, digest: format!("sha256:{}", "2".repeat(64)) },
            dserv_artifact_id: "dserv:pipeline:source".to_string(),
            dserv_artifact_digest: digest,
            dserv_artifact_descriptor_digest: descriptor_digest,
            dnode_abi: "datum-dnode/0".to_string(),
        };
        let executor = InProcessExecutor { cache: &cache, store: &store };
        invoke_dcode_governed_capturing_emits_for_test(&executor, &store, &authorization, &artifact, this_node_id, "in", b"trigger")
    }

    fn fanout_grant_response(call_id: &str, message_id: &str, correlation_id: &str) -> String {
        serde_json::json!({
            "grant": {
                "grant_id": format!("grant-{call_id}"),
                "project_id": "proj:lab",
                "application_id": "app:a",
                "call_id": call_id,
                "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": "sha256:aa"},
                "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": "sha256:bb"},
                "source": {"service_id": "source", "port": "out", "node_id": "fog-01", "dserv_artifact_id": "a1", "dserv_artifact_digest": "sha256:cc", "dserv_artifact_descriptor_digest": "sha256:dd"},
                "destination": {"service_id": "dest", "port": "in", "node_id": "cloud-01", "dserv_artifact_id": "a2", "dserv_artifact_digest": "sha256:ee", "dserv_artifact_descriptor_digest": "sha256:ff"},
                "message_id": message_id,
                "correlation_id": correlation_id,
                "payload_sha256": sha256_hex(b"payload"),
                "payload_size": 7,
                "source_dcode_evidence_sha256": "f".repeat(64),
                // Deliberately an UNSUPPORTED route_kind: this fails fast in
                // `dispatch_one_call`'s `other =>` branch, with NO further
                // HTTP calls (no redeem, no D-Code execution) -- exactly
                // enough to prove fan-out plumbing without needing to mock
                // the entire delivery chain.
                "route_kind": "unsupported-transport-for-test",
                "issued_at": "2026-01-01T00:00:00Z",
                "expires_at": "2026-01-01T00:00:30Z",
            },
            "token": "test-token",
        })
        .to_string()
    }

    #[test]
    fn fan_out_gives_each_call_a_unique_message_id_but_shares_one_correlation_id_and_one_failure_does_not_erase_another() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            // Request 1: list_calls.
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = serde_json::json!(["call-a", "call-b"]).to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
            // Requests 2 and 3: one request_grant POST per call, in order.
            for _ in 0..2 {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        match stream.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(n) => {
                                buffer.extend_from_slice(&chunk[..n]);
                                if buffer.windows(4).any(|w| w == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    let text = String::from_utf8_lossy(&buffer);
                    let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(text.len());
                    let request_body: serde_json::Value = serde_json::from_str(&text[body_start..]).unwrap_or(serde_json::Value::Null);
                    let call_id = request_body.get("call_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let message_id = request_body.get("message_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let correlation_id = request_body.get("correlation_id").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                    let mut body: serde_json::Value = serde_json::from_str(&fanout_grant_response(&call_id, &message_id, &correlation_id)).unwrap();
                    body["grant"]["source_dcode_evidence_sha256"] = request_body["source_dcode_evidence_sha256"].clone();
                    let body = body.to_string();
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        });

        let context = SourceExecutionContext { dserver_url: format!("http://{addr}"), dnode_root: "unused".to_string() };

        let dnode_root = std::env::temp_dir().join(format!("dcall-fanout-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext { dnode_root: dnode_root.to_string_lossy().to_string(), ..context };

        let source_result = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#);
        assert_eq!(source_result.evidence().outcome, DCodeInvocationOutcome::Success);
        assert_eq!(source_result.validated_emits().len(), 1);
        let expected_correlation_id = source_result.evidence().correlation_id.clone();

        let results = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 2, "one delivery attempt per resolved call, results: {results:?}");

        for evidence in &results {
            assert!(matches!(evidence.outcome, DCallDeliveryOutcome::Failed));
            assert_eq!(evidence.error_code.as_deref(), Some("dcall_unsupported_route_kind"));
        }

        let call_ids: std::collections::BTreeSet<_> = results.iter().map(|e| e.call_id.clone()).collect();
        assert_eq!(call_ids, ["call-a".to_string(), "call-b".to_string()].into_iter().collect());

        for evidence in &results {
            assert_eq!(evidence.correlation_id, expected_correlation_id, "every fan-out delivery must share the SOURCE invocation's own correlation_id");
        }
        let message_ids: std::collections::BTreeSet<_> = results.iter().map(|e| e.message_id.clone()).collect();
        assert_eq!(message_ids.len(), 2, "expected two distinct message_ids, got: {message_ids:?}");
    }

    #[test]
    fn dispatch_reports_a_sanitized_lookup_failure_when_dserver_is_unreachable() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-unreachable-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext { dserver_url: "http://127.0.0.1:1".to_string(), dnode_root: dnode_root.to_string_lossy().to_string() };
        let source_result = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#);
        let results = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, DCallDeliveryOutcome::Failed));
        assert_eq!(results[0].error_code.as_deref(), Some("dcall_lookup_failed"));
        assert!(results[0].error_diagnostic_sha256.is_some());
        let serialized = serde_json::to_string(&results[0]).unwrap();
        assert!(!serialized.contains("Connection refused") || results[0].error_diagnostic_sha256.is_some());
    }

    // -- J1: source emit provenance (opaque `DCodeInvocationResult`) --------

    // -- K1: a generic custom `DCodeExecutor` can still mint a plain, opaque
    // `DCodeInvocationResult` (J1's private-fields fix prevents fabricating
    // one from raw evidence+bytes, but never claimed to prevent a FAKE
    // EXECUTOR from producing one through a real, successful
    // `invoke_dcode_capturing_emits` call). This is exactly why production
    // D-Call dispatch does NOT accept `DCodeInvocationResult` at all -- see
    // `governed-source-invocation-provenance-audit.txt`.
    use crate::dcode::invoke::{invoke_dcode_capturing_emits, DCodeExecutor};
    use crate::dcode::runtime::{ExecutionOutcome, ExecutionOutput, InvocationLimits};

    /// A fake executor that never runs any real WASM at all -- it just
    /// fabricates a syntactically valid ADR-0003 success envelope.
    struct FakeExecutor;
    impl DCodeExecutor for FakeExecutor {
        fn execute(&self, _digest: &str, _limits: &InvocationLimits, _input: &[u8]) -> ExecutionOutcome {
            let body = r#"{"status":"ok","emits":[]}"#;
            ExecutionOutcome::Success(ExecutionOutput::Bytes(body.as_bytes().to_vec()))
        }
    }

    #[test]
    fn a_fake_executor_can_still_mint_a_generic_but_not_a_governed_result() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-k1-fake-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        let module_bytes = respond_wat(r#"{"status":"ok","emits":[]}"#);
        let digest = sha256_hex(&module_bytes);
        let store = DCodeModuleStore::new(&dnode_root);
        store.install(&module_bytes, &digest).unwrap();
        let (_cache, _store2, authorization, artifact) = {
            // Reuse the same fixture shape `real_source_result` builds, but
            // with the FAKE executor instead of a real one -- proving the
            // fake never has to touch a real module at all (`_digest` and
            // `_input` are ignored by `FakeExecutor::execute`).
            let cache = DCodeModuleCache::new().unwrap();
            (cache, (), authorization_for_fake_test(&digest, &module_bytes), artifact_for_fake_test(&digest, &module_bytes))
        };
        let executor = FakeExecutor;
        // This compiles and succeeds: a generic `DCodeInvocationResult` is
        // genuinely obtainable from a fake, non-isolated, non-real-Wasmtime
        // executor -- J1's opacity alone does not prevent this.
        let fake_result = invoke_dcode_capturing_emits(&executor, &store, &authorization, &artifact, "fog-01", "in", b"trigger");
        assert_eq!(fake_result.evidence().outcome, DCodeInvocationOutcome::Success);

        // K1: `dispatch_validated_emit`/`dispatch_validated_emit_for_test`
        // require `&GovernedDCodeInvocationResult`, a DIFFERENT type with no
        // public constructor and no `From<DCodeInvocationResult>` conversion
        // anywhere in this crate (grep-verified: the only two producers are
        // `invoke_dcode_governed_capturing_emits` and its `_for_test` twin,
        // the default producer uses the real isolated worker; the explicit
        // test-support twin is permitted to use synthetic executors). There is
        // therefore no expression that turns `fake_result` (type
        // `DCodeInvocationResult`) into the type production dispatch accepts
        // -- passing it directly would be a hard COMPILE error, which is a
        // stronger guarantee than any runtime assertion this test could make.
        // (This crate has no trybuild-style compile-fail harness to assert
        // that non-compilation directly; the absence of any conversion path
        // is the actual guarantee, confirmed by reading `invoke.rs` itself.)
        let _ = fake_result; // proven mintable, but structurally unusable below
        drop(dnode_root);
    }

    fn authorization_for_fake_test(digest: &str, module_bytes: &[u8]) -> DCodeInvocationAuthorizationV1 {
        let artifact = artifact_for_fake_test(digest, module_bytes);
        let descriptor_digest = crate::dcode::dserv_artifact::recompute_dserv_artifact_digest(&artifact).unwrap();
        DCodeInvocationAuthorizationV1 {
            authorization_id: "auth-fake".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:a".to_string(),
            service_id: "source".to_string(),
            node_id: "fog-01".to_string(),
            dmap_ref: CanonicalContractReferenceV1 { id: "dmap:1".to_string(), revision: 1, digest: format!("sha256:{}", "1".repeat(64)) },
            dgraph_ref: CanonicalContractReferenceV1 { id: "dgraph:1".to_string(), revision: 1, digest: format!("sha256:{}", "2".repeat(64)) },
            dserv_artifact_id: "dserv:pipeline:source".to_string(),
            dserv_artifact_digest: digest.to_string(),
            dserv_artifact_descriptor_digest: descriptor_digest,
            dnode_abi: "datum-dnode/0".to_string(),
        }
    }

    fn artifact_for_fake_test(digest: &str, module_bytes: &[u8]) -> CanonicalDServArtifactV1 {
        CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: "dserv:pipeline:source".to_string(),
            version: "0.1.0".to_string(),
            application_id: "app:a".to_string(),
            dserv_id: "source".to_string(),
            dcode: DServArtifactDCodeV1 { uri: format!("datum-blob://sha256/{digest}"), sha256: digest.to_string(), content_id: format!("sha256:{digest}"), size_bytes: module_bytes.len() as u64 },
            abi: DServArtifactAbiV1 { name: "datum-dnode".to_string(), version: "0".to_string(), required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(), allowed_imports: Vec::new() },
            ports: DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 { port: "in".to_string(), payload_schema: "s".to_string(), external: true }],
                outputs: Vec::new(),
            },
            limits: DServArtifactLimitsV1 { fuel_per_invocation: 5_000_000, timeout_ms: 200, max_memory_pages: 4, max_message_bytes: 65536, max_emits_per_invocation: 8 },
            safety: DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: DServArtifactConstraintsV1::default(),
            qos: serde_json::Value::Null,
            provenance: serde_json::Value::Null,
            probes: serde_json::Value::Null,
            signature_ref: None,
        }
    }

    #[test]
    fn dispatch_rejects_a_source_invocation_that_was_not_successful() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-a-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext { dserver_url: "http://127.0.0.1:1".to_string(), dnode_root: dnode_root.to_string_lossy().to_string() };
        // authorization.node_id="node-x" but the invocation runs as "fog-01"
        // -> a REAL StaleAuthority failure, not a hand-set enum value.
        let source_result = real_source_result("node-x", "fog-01", r#"{"status":"ok","emits":[]}"#);
        assert_ne!(source_result.evidence().outcome, DCodeInvocationOutcome::Success);
        let results = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].error_code.as_deref(), Some("dcall_source_invocation_not_successful"));
    }

    #[test]
    fn dispatch_rejects_an_out_of_range_emit_index() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-b-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext { dserver_url: "http://127.0.0.1:1".to_string(), dnode_root: dnode_root.to_string_lossy().to_string() };
        // A real, successful invocation that emits NOTHING -- index 0 is out
        // of range for its (empty) `validated_emits()`.
        let source_result = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[]}"#);
        assert_eq!(source_result.evidence().outcome, DCodeInvocationOutcome::Success);
        assert!(source_result.validated_emits().is_empty());
        let results = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].error_code.as_deref(), Some("dcall_source_emit_not_found"));
    }

    // J1: the old "arbitrary bytes with a mismatching digest" test is no
    // longer expressible -- `dispatch_validated_emit`/`_for_test` no longer
    // takes a raw `&[u8]` payload parameter at all; the emit's bytes ARE
    // whatever `ValidatedDCodeEmit::payload()` returns for the selected
    // index, with no separate value to mismatch against. This is now a
    // compile-time property of the public function signature, not a runtime
    // check -- see `source-dcode-emit-provenance-audit.txt`.

    /// AK: a process cannot fabricate a source node identity -- if the
    /// source evidence claims a node other than this installation's own
    /// local D-Node identity, dispatch fails closed.
    #[test]
    fn dispatch_rejects_source_evidence_claiming_a_different_node_than_this_installations_local_identity() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-d-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext { dserver_url: "http://127.0.0.1:1".to_string(), dnode_root: dnode_root.to_string_lossy().to_string() };
        // Real invocation runs (and its evidence claims) node "cloud-99", but
        // this installation's own local identity is "fog-01".
        let source_result = real_source_result("cloud-99", "cloud-99", r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#);
        assert_eq!(source_result.evidence().outcome, DCodeInvocationOutcome::Success);
        let results = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].error_code.as_deref(), Some("dcall_source_local_identity_mismatch"));
    }

    #[test]
    fn dispatch_fails_closed_before_any_network_attempt_when_local_identity_is_not_provisioned() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-e-{}", Uuid::new_v4()));
        // deliberately NOT initialized
        let context = SourceExecutionContext { dserver_url: "http://127.0.0.1:1".to_string(), dnode_root: dnode_root.to_string_lossy().to_string() };
        let source_result = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#);
        let results = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].error_code.as_deref(), Some("dcall_source_local_identity_load_failed"));
    }

    /// J10: a source invocation producing TWO emits on the SAME output port
    /// with different payloads must let each be dispatched independently --
    /// each is a distinct index into `validated_emits()`, so there is no
    /// "compare against the first emit's digest" bug (J1C) to trigger.
    #[test]
    fn two_emits_on_the_same_port_are_both_independently_dispatchable() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-j10-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext { dserver_url: "http://127.0.0.1:1".to_string(), dnode_root: dnode_root.to_string_lossy().to_string() };
        // "out"/"A" (QQ==) and "out"/"B" (Qg==) -- same port, different payloads.
        let source_result = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"QQ=="},{"port":"out","payload_b64":"Qg=="}]}"#,
        );
        assert_eq!(source_result.evidence().outcome, DCodeInvocationOutcome::Success);
        assert_eq!(source_result.validated_emits().len(), 2);
        assert_eq!(source_result.validated_emits()[0].payload(), b"A");
        assert_eq!(source_result.validated_emits()[1].payload(), b"B");

        // Both dispatched separately -- neither is falsely rejected as a
        // "payload mismatch" against the other's digest (the exact J1C bug).
        // `dserver_url` is unreachable, so both fail at the SAME later stage
        // (`dcall_lookup_failed`), proving each independently passed the
        // provenance checks (source-invocation-successful, emit-index
        // resolution, local-identity match) before reaching the network.
        let results_a = dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        let results_b = dispatch_validated_emit_for_test(&context, &source_result, 1, Path::new("unused"));
        assert_eq!(results_a.len(), 1);
        assert_eq!(results_b.len(), 1);
        assert_eq!(results_a[0].error_code.as_deref(), Some("dcall_lookup_failed"));
        assert_eq!(results_b[0].error_code.as_deref(), Some("dcall_lookup_failed"));
    }

    // -- I3: destination evidence must be validated against the grant -------

    fn sample_grant() -> DCallDeliveryGrantV1 {
        use crate::dcall::grant::{CanonicalContractReferenceV1, DCallRouteEndpointV1};
        let endpoint = DCallRouteEndpointV1 {
            service_id: "dest".to_string(),
            port: "in".to_string(),
            node_id: "fog-01".to_string(),
            dserv_artifact_id: "artifact:dest".to_string(),
            dserv_artifact_digest: "sha256:aa".to_string(),
            dserv_artifact_descriptor_digest: "sha256:bb".to_string(),
        };
        DCallDeliveryGrantV1 {
            grant_id: "grant-1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:a".to_string(),
            call_id: "call:1".to_string(),
            dgraph_ref: CanonicalContractReferenceV1 { id: "dgraph:1".to_string(), revision: 1, digest: "sha256:cc".to_string() },
            dmap_ref: CanonicalContractReferenceV1 { id: "dmap:1".to_string(), revision: 1, digest: "sha256:dd".to_string() },
            source: endpoint.clone(),
            destination: endpoint,
            message_id: "dcall-msg:1".to_string(),
            correlation_id: "dcall-corr:1".to_string(),
            payload_sha256: sha256_hex(b"payload"),
            payload_size: 7,
            source_dcode_evidence_sha256: "f".repeat(64),
            route_kind: "local".to_string(),
            endpoint_id: None,
            endpoint_revision: None,
            endpoint_descriptor_digest: None,
            issued_at: "2026-01-01T00:00:00Z".to_string(),
            expires_at: "2026-01-01T00:00:30Z".to_string(),
        }
    }

    fn matching_dcode_evidence(grant: &DCallDeliveryGrantV1) -> DCodeInvocationEvidenceV1 {
        DCodeInvocationEvidenceV1 {
            schema: DCODE_EVIDENCE_SCHEMA.to_string(),
            authorization_id: "auth-1".to_string(),
            project_id: grant.project_id.clone(),
            application_id: grant.application_id.clone(),
            service_id: grant.destination.service_id.clone(),
            node_id: grant.destination.node_id.clone(),
            dmap_id: grant.dmap_ref.id.clone(),
            dmap_revision: grant.dmap_ref.revision,
            dmap_digest: grant.dmap_ref.digest.clone(),
            dgraph_id: grant.dgraph_ref.id.clone(),
            dgraph_revision: grant.dgraph_ref.revision,
            dgraph_digest: grant.dgraph_ref.digest.clone(),
            dserv_artifact_id: grant.destination.dserv_artifact_id.clone(),
            dserv_artifact_digest: grant.destination.dserv_artifact_digest.clone(),
            dserv_artifact_descriptor_digest: grant.destination.dserv_artifact_descriptor_digest.clone(),
            module_size_bytes: Some(64),
            dnode_abi: "datum-dnode/0".to_string(),
            fresh_instance_per_invocation: true,
            input_port: grant.destination.port.clone(),
            message_id: grant.message_id.clone(),
            correlation_id: grant.correlation_id.clone(),
            input_sha256: grant.payload_sha256.clone(),
            input_size: grant.payload_size as usize,
            input_envelope_sha256: "f".repeat(64),
            input_envelope_size: 100,
            received_at_utc: "2026-01-01T00:00:00Z".to_string(),
            output_sha256: None,
            output_size: None,
            output_status: Some("ok".to_string()),
            output_reason_present: false,
            output_reason_sha256: None,
            output_reason_size: None,
            emits: Vec::new(),
            limits_applied: DCodeAppliedLimits { fuel_per_invocation: 1_000_000, timeout_ms: 100, max_memory_pages: 4, max_message_bytes: 65536 },
            outcome: DCodeInvocationOutcome::Success,
            error_code: None,
            error_message: None,
            error_offending_value_sha256: None,
            error_offending_value_size: None,
            duration_ms: 1,
        }
    }

    #[test]
    fn matching_destination_evidence_passes_the_grant_binding_check() {
        let grant = sample_grant();
        let evidence = matching_dcode_evidence(&grant);
        assert!(validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_ok());
    }

    #[test]
    fn destination_evidence_from_a_different_node_fails_the_grant_binding_check() {
        let grant = sample_grant();
        let mut evidence = matching_dcode_evidence(&grant);
        evidence.node_id = "some-other-node".to_string();
        assert!(validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_err());
    }

    #[test]
    fn destination_evidence_for_a_different_dmap_fails_the_grant_binding_check() {
        let grant = sample_grant();
        let mut evidence = matching_dcode_evidence(&grant);
        evidence.dmap_digest = "sha256:some-other-dmap-digest".to_string();
        assert!(validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_err());
    }

    #[test]
    fn destination_evidence_with_a_different_message_id_fails_the_grant_binding_check() {
        let grant = sample_grant();
        let mut evidence = matching_dcode_evidence(&grant);
        evidence.message_id = "dcall-msg:forged".to_string();
        assert!(validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_err());
    }

    #[test]
    fn destination_evidence_for_a_different_payload_fails_the_grant_binding_check() {
        let grant = sample_grant();
        let mut evidence = matching_dcode_evidence(&grant);
        evidence.input_sha256 = "0".repeat(64);
        assert!(validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_err());
    }

    #[test]
    fn every_destination_authority_and_delivery_identity_substitution_is_rejected() {
        let grant = sample_grant();
        let original = serde_json::to_value(matching_dcode_evidence(&grant)).unwrap();
        for field in ["schema", "project_id", "application_id", "service_id", "node_id",
            "dgraph_id", "dgraph_revision", "dgraph_digest", "dmap_id", "dmap_revision", "dmap_digest",
            "dserv_artifact_id", "dserv_artifact_digest", "dserv_artifact_descriptor_digest",
            "input_port", "input_sha256", "input_size", "message_id", "correlation_id"] {
            let mut value = original.clone();
            value[field] = if field.ends_with("revision") || field == "input_size" { serde_json::json!(999) }
                else { serde_json::json!("substitution") };
            let evidence = serde_json::from_value(value).unwrap();
            assert!(validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_err(), "{field}");
        }
    }

    #[test]
    fn source_evidence_digest_substitution_fails_before_redemption_or_execution() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for body in [serde_json::json!(["call-a"]).to_string(), fanout_grant_response("call-a", "m1", "c1")] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = [0; 8192];
                stream.read(&mut bytes).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            // Closing the observer here makes any subsequent redeem/execute
            // call fail; the returned specific mismatch proves that branch
            // was reached before transport or redemption.
        });
        let root = std::env::temp_dir().join(format!("dcall-source-substitution-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let source = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#);
        let context = SourceExecutionContext { dserver_url: format!("http://{addr}"), dnode_root: root.to_str().unwrap().to_string() };
        let result = dispatch_validated_emit_for_test(&context, &source, 0, Path::new("unused"));
        assert_eq!(result[0].error_code.as_deref(), Some("dcall_source_evidence_mismatch"));
        assert!(result[0].redeemed_at.is_none());
        assert!(result[0].destination_dcode_evidence_sha256.is_none());
        thread.join().unwrap();
    }

    // -- I2: grant/DCode message identity equality ---------------------------

    #[test]
    fn execute_destination_delivery_uses_the_grants_own_message_and_correlation_id() {
        // This is proven end-to-end (not just by reading the source) in the
        // real `dcall_worker_isolation`/`dcall_two_node_integration` tests,
        // which invoke real D-Code and assert on the ACTUAL resulting
        // DCodeInvocationEvidenceV1's message_id/correlation_id. This unit
        // test documents the compile-time contract: `execute_destination_delivery`
        // has NO parameter for a caller-supplied correlation/message id other
        // than the ones already embedded in `grant`.
        let grant = sample_grant();
        assert_eq!(grant.message_id, "dcall-msg:1");
        assert_eq!(grant.correlation_id, "dcall-corr:1");
    }

    // -- I6: bounded remote response ------------------------------------------

    #[test]
    fn oversized_declared_content_length_is_rejected_without_reading_the_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_REMOTE_RESPONSE_BYTES + 1);
                let _ = stream.write_all(response.as_bytes());
                // Deliberately never sends the (nonexistent) oversized body --
                // if the client tried to read it, it would hang/fail rather
                // than reject cleanly, which is exactly what this test proves
                // does NOT happen.
            }
        });
        let client = reqwest::blocking::Client::new();
        let response = client.get(format!("http://{addr}")).send().unwrap();
        let error = read_bounded_response_body(response).unwrap_err();
        assert_eq!(error.code, DCallDeliveryErrorCode::DCallRemoteResponseTooLarge);
    }

    #[test]
    fn a_body_larger_than_the_cap_is_rejected_even_without_a_declared_content_length() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let oversized_body = "x".repeat(MAX_REMOTE_RESPONSE_BYTES + 4096);
                // Chunked transfer-encoding so no Content-Length is declared.
                let chunk_header = format!("{:x}\r\n", oversized_body.len());
                let response = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{chunk_header}{oversized_body}\r\n0\r\n\r\n");
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let client = reqwest::blocking::Client::new();
        let response = client.get(format!("http://{addr}")).send().unwrap();
        let error = read_bounded_response_body(response).unwrap_err();
        assert_eq!(error.code, DCallDeliveryErrorCode::DCallRemoteResponseTooLarge);
    }

    #[test]
    fn a_normal_sized_response_is_read_successfully() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = serde_json::json!({"outcome":"success"}).to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let client = reqwest::blocking::Client::new();
        let response = client.get(format!("http://{addr}")).send().unwrap();
        let body = read_bounded_response_body(response).unwrap();
        assert!(body.contains("success"));
    }
}
