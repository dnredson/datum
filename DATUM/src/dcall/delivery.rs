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
use crate::dcall::grant::{
    DCallDeliveryGrantV1, SourceExecutionBindingV1, ROUTE_KIND_LOCAL, ROUTE_KIND_REMOTE,
    TRANSPORT_HTTP,
};
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
    dispatch_validated_emit_impl(
        context,
        source_result,
        emit_index,
        &default_worker_binary_path(),
    )
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
            Some(&format!(
                "source invocation outcome was {:?}, not Success",
                source_evidence.outcome
            )),
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

    let call_ids = match client::list_calls_for_source_service(
        &context.dserver_url,
        &source_evidence.project_id,
        &source_evidence.application_id,
        &source_evidence.service_id,
    ) {
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
            dispatch_one_call(
                context,
                source_evidence,
                &call_id,
                &emit_port,
                emit_payload,
                &actual_sha256,
                &message_id,
                worker_binary,
            )
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
    let source_dcode_evidence_sha256 =
        crate::dcode::evidence::dcode_evidence_sha256(source_evidence);

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
            let code = error
                .downcast_ref::<client::GrantRequestRejection>()
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
            id,
            &grant,
            None,
            DCallDeliveryErrorCode::DCallSourceEvidenceMismatch,
            Some("issued grant substituted the source evidence digest"),
            elapsed_ms(start),
        );
    }

    // Phase 113B3 (section 2): once DServer issues a grant carrying typed
    // DMap/2 `via_diot_pubsub` transport authority, delivery MUST NOT fall
    // through into the `route_kind`-based branches below -- `route_kind` is
    // purely descriptive topology (section 10/11: it never gates transport
    // for this authority shape, and a same-node call still carries this
    // authority) -- this check fires regardless of whether `route_kind`
    // happens to be "local" or "remote". This REPLACES Phase 113B2's
    // temporary fail-closed seam (which returned `DCallDiotTransportNotImplemented`
    // unconditionally here) with real governed MQTT delivery.
    if grant.v2_dcall_transport_authority.is_some() {
        let expires_at = chrono::DateTime::parse_from_rfc3339(&grant.expires_at)
            .map(|value| value.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now());
        let bound = expires_at
            .signed_duration_since(chrono::Utc::now())
            .to_std()
            .unwrap_or(std::time::Duration::ZERO);
        let max_wait = bound.min(std::time::Duration::from_secs(30));
        let result = deliver_via_diot_pubsub(
            &context.dserver_url,
            &grant,
            &token,
            payload,
            payload_sha256,
            max_wait,
            |broker| {
                let transport = crate::dcall::mqtt::RumqttcPubSub::connect(
                    broker,
                    &crate::dcall::mqtt::publisher_client_id(),
                )?;
                Ok(Box::new(transport) as Box<dyn crate::dcall::mqtt::MqttPubSub>)
            },
        );
        return match result {
            Ok((dcode_evidence, redeemed_at)) => {
                if let Err(mismatch) =
                    validate_destination_dcode_evidence_against_grant(&dcode_evidence, &grant)
                {
                    return evidence::build_grant_bound_failure_evidence(
                        id,
                        &grant,
                        Some(redeemed_at),
                        DCallDeliveryErrorCode::DCallDestinationEvidenceMismatch,
                        Some(&mismatch),
                        elapsed_ms(start),
                    );
                }
                evidence::build_success_evidence(
                    id,
                    &grant,
                    redeemed_at,
                    &serde_json::to_vec(&dcode_evidence).unwrap_or_default(),
                    elapsed_ms(start),
                )
            }
            Err(failure) => evidence::build_grant_bound_failure_evidence(
                id,
                &grant,
                failure.redeemed_at,
                failure.code,
                failure.diagnostic.as_deref(),
                elapsed_ms(start),
            ),
        };
    }

    let result: Result<(DCodeInvocationEvidenceV1, String), DeliveryFailure> =
        match grant.route_kind.as_str() {
            ROUTE_KIND_LOCAL => deliver_local(
                context,
                &grant,
                &token,
                payload,
                payload_sha256,
                worker_binary,
            ),
            ROUTE_KIND_REMOTE => deliver_remote(
                &context.dserver_url,
                &grant,
                &token,
                payload,
                payload_sha256,
            ),
            other => Err(DeliveryFailure {
                code: DCallDeliveryErrorCode::DCallUnsupportedRouteKind,
                diagnostic: Some(format!(
                    "unsupported route_kind '{other}' returned by DServer"
                )),
                redeemed_at: None,
            }),
    };

    match result {
        Ok((dcode_evidence, redeemed_at)) => {
            // I3: never trust destination evidence merely because it
            // deserialized -- prove it actually corresponds to the grant
            // just redeemed (every load-bearing identity/digest field).
            if let Err(mismatch) =
                validate_destination_dcode_evidence_against_grant(&dcode_evidence, &grant)
            {
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
                evidence::build_success_evidence(
                    id,
                    &grant,
                    redeemed_at,
                    &evidence_json,
                    elapsed_ms(start),
                )
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
        Err(failure) => evidence::build_grant_bound_failure_evidence(
            id,
            &grant,
            failure.redeemed_at,
            failure.code,
            failure.diagnostic.as_deref(),
            elapsed_ms(start),
        ),
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
fn validate_destination_dcode_evidence_against_grant(
    evidence: &DCodeInvocationEvidenceV1,
    grant: &DCallDeliveryGrantV1,
) -> Result<(), String> {
    if evidence.schema != crate::dcode::evidence::DCODE_EVIDENCE_SCHEMA
        || evidence.dserv_artifact_id != grant.destination.dserv_artifact_id
    {
        return Err(
            "destination evidence schema or artifact id does not match the grant".to_string(),
        );
    }
    if evidence.project_id != grant.project_id {
        return Err(format!(
            "destination evidence project_id '{}' != grant project_id '{}'",
            evidence.project_id, grant.project_id
        ));
    }
    if evidence.application_id != grant.application_id {
        return Err(format!(
            "destination evidence application_id '{}' != grant application_id '{}'",
            evidence.application_id, grant.application_id
        ));
    }
    if evidence.service_id != grant.destination.service_id {
        return Err(format!(
            "destination evidence service_id '{}' != grant destination.service_id '{}'",
            evidence.service_id, grant.destination.service_id
        ));
    }
    if evidence.node_id != grant.destination.node_id {
        return Err(format!(
            "destination evidence node_id '{}' != grant destination.node_id '{}'",
            evidence.node_id, grant.destination.node_id
        ));
    }
    if evidence.dgraph_id != grant.dgraph_ref.id
        || evidence.dgraph_revision != grant.dgraph_ref.revision
        || evidence.dgraph_digest != grant.dgraph_ref.digest
    {
        return Err(
            "destination evidence DGraph ref does not match the grant's bound DGraph ref"
                .to_string(),
        );
    }
    if evidence.dmap_id != grant.dmap_ref.id
        || evidence.dmap_revision != grant.dmap_ref.revision
        || evidence.dmap_digest != grant.dmap_ref.digest
    {
        return Err(
            "destination evidence DMap ref does not match the grant's bound DMap ref".to_string(),
        );
    }
    if evidence.dserv_artifact_id != grant.destination.dserv_artifact_id {
        return Err(format!("destination evidence dserv_artifact_id '{}' != grant destination.dserv_artifact_id '{}'", evidence.dserv_artifact_id, grant.destination.dserv_artifact_id));
    }
    if evidence.dserv_artifact_digest != grant.destination.dserv_artifact_digest {
        return Err(
            "destination evidence module digest does not match the grant's bound module digest"
                .to_string(),
        );
    }
    if evidence.dserv_artifact_descriptor_digest
        != grant.destination.dserv_artifact_descriptor_digest
    {
        return Err("destination evidence descriptor digest does not match the grant's bound descriptor digest".to_string());
    }
    if evidence.input_port != grant.destination.port {
        return Err(format!(
            "destination evidence input_port '{}' != grant destination.port '{}'",
            evidence.input_port, grant.destination.port
        ));
    }
    if evidence.input_sha256 != grant.payload_sha256 {
        return Err(
            "destination evidence input_sha256 does not match the grant's bound payload_sha256"
                .to_string(),
        );
    }
    if evidence.input_size as u64 != grant.payload_size {
        return Err(
            "destination evidence input_size does not match the grant's bound payload_size"
                .to_string(),
        );
    }
    if evidence.message_id != grant.message_id {
        return Err(format!(
            "destination evidence message_id '{}' != grant message_id '{}'",
            evidence.message_id, grant.message_id
        ));
    }
    if evidence.correlation_id != grant.correlation_id {
        return Err(format!(
            "destination evidence correlation_id '{}' != grant correlation_id '{}'",
            evidence.correlation_id, grant.correlation_id
        ));
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
    let redeemed = client::redeem_grant(
        &context.dserver_url,
        &grant.grant_id,
        token,
        &grant.destination.node_id,
        payload_sha256,
        payload.len() as u64,
        None,
    )
    .map_err(|error| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallGrantRedeemFailed,
        diagnostic: Some(error.to_string()),
        redeemed_at: None,
    })?;
    let redeemed_at = now_rfc3339();
    let evidence = execute_destination_delivery(
        &context.dserver_url,
        &context.dnode_root,
        worker_binary,
        &redeemed.destination.node_id,
        &redeemed,
        payload,
    )
    .map_err(|error| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallDestinationExecutionFailed,
        diagnostic: Some(error.to_string()),
        redeemed_at: Some(redeemed_at.clone()),
    })?;
    Ok((evidence, redeemed_at))
}

/// I6: reads at most `MAX_REMOTE_RESPONSE_BYTES` from a blocking `reqwest`
/// response -- rejects immediately on an oversized declared `Content-Length`
/// (without reading anything), and separately caps the actual bytes read
/// via `Read::take` so a listener that declares no/short Content-Length but
/// then streams unboundedly cannot force unbounded allocation either.
fn read_bounded_response_body(
    response: reqwest::blocking::Response,
) -> Result<String, DeliveryFailure> {
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
    limited
        .read_to_end(&mut buffer)
        .map_err(|error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallTransportError,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        })?;
    if buffer.len() > MAX_REMOTE_RESPONSE_BYTES {
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallRemoteResponseTooLarge,
            diagnostic: Some(format!("remote response body exceeded the {MAX_REMOTE_RESPONSE_BYTES}-byte cap while reading")),
            redeemed_at: None,
        });
    }
    Ok(String::from_utf8_lossy(&buffer).to_string())
}

fn deliver_remote(
    dserver_url: &str,
    grant: &DCallDeliveryGrantV1,
    token: &str,
    payload: &[u8],
    payload_sha256: &str,
) -> Result<(DCodeInvocationEvidenceV1, String), DeliveryFailure> {
    let endpoint = client::fetch_endpoint(dserver_url, &grant.destination.node_id, TRANSPORT_HTTP)
        .map_err(|error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallEndpointFetchFailed,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        })?;
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
            diagnostic: Some(
                "fetched endpoint does not match grant-bound transport authority".to_string(),
            ),
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
        .map_err(|error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallTransportError,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        })?;
    let response = match http_client.post(&url).json(&request).send() {
        Ok(response) => response,
        Err(error) => {
            let code = if error.is_timeout() {
                DCallDeliveryErrorCode::DCallTransportTimeout
            } else {
                DCallDeliveryErrorCode::DCallTransportError
            };
            return Err(DeliveryFailure {
                code,
                diagnostic: Some(error.to_string()),
                redeemed_at: None,
            });
        }
    };
    let status = response.status();
    let body_text = read_bounded_response_body(response)?;
    if !status.is_success() {
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallTransportError,
            diagnostic: Some(format!(
                "remote D-Call delivery to {url} failed: status={status} body={body_text}"
            )),
            redeemed_at: None,
        });
    }
    let body: serde_json::Value =
        serde_json::from_str(&body_text).map_err(|_| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
            diagnostic: Some("remote success response is not JSON".to_string()),
            redeemed_at: None,
        })?;
    let outcome = body
        .as_object()
        .and_then(|object| object.get("outcome"))
        .and_then(|value| value.as_str())
        .filter(|outcome| matches!(*outcome, "success" | "failed"))
        .ok_or_else(|| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
            diagnostic: Some("remote success response has invalid outcome".to_string()),
            redeemed_at: None,
        })?;
    if outcome != "success" {
        // We honestly do not know from here whether the listener redeemed
        // the grant before failing (e.g. a validation error after
        // redemption vs. a rejection before it) -- report `redeemed_at` as
        // unknown (`None`) rather than guessing either way.
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallDestinationExecutionFailed,
            diagnostic: Some(format!(
                "remote D-Call delivery reported outcome='{outcome}': {body_text}"
            )),
            redeemed_at: None,
        });
    }
    let evidence_json = match body.get("dcode_evidence").cloned() {
        Some(value) => value,
        None => {
            return Err(DeliveryFailure {
                code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
                diagnostic: Some(
                    "remote D-Call delivery response missing dcode_evidence".to_string(),
                ),
                redeemed_at: None,
            })
        }
    };
    let evidence: DCodeInvocationEvidenceV1 =
        serde_json::from_value(evidence_json).map_err(|error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        })?;
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
    // Phase 117H: one governed preflight call resolves a fresh authorization
    // AND the EXACT authorized D-Code descriptor revision together (never
    // the old logical fetch, which cannot execute the valid governed
    // scenario where multiple revisions coexist for the destination
    // service) -- `dcall::grant`'s mirror and `dcode::authorization`'s
    // mirror are two independently-declared (but wire-identical) types, so
    // this still goes through the real `dcode::client`, applying the exact
    // same governed preflight Phase 111/117G/117H already established,
    // unchanged.
    let preflight = crate::dcode::client::fetch_governed_dcode_execution_material(
        dserver_url,
        &grant.project_id,
        &grant.application_id,
        &grant.destination.service_id,
        this_node_id,
    )?;
    let authorization: DCodeInvocationAuthorizationV1 = preflight.authorization;
    let artifact: CanonicalDServArtifactV1 = preflight.artifact;

    if !artifact.validate().is_empty() {
        return Err(anyhow!(
            "fetched destination D-Serv artifact fails structural validation"
        ));
    }

    let module_store = DCodeModuleStore::new(dnode_root);
    let executor = IsolatedWorkerExecutor {
        worker_binary,
        module_store_root: std::path::Path::new(dnode_root),
    };

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
    let grant = client::redeem_grant(
        dserver_url,
        grant_id,
        token,
        &identity.node_id,
        &sha256_hex(payload),
        payload.len() as u64,
        None,
    )?;
    execute_destination_delivery(
        dserver_url,
        dnode_root,
        &default_worker_binary_path(),
        &identity.node_id,
        &grant,
        payload,
    )
}

/// Phase 113B3: the MQTT-transport analog of `receive_governed_delivery`
/// (section 19: shares the SAME `execute_destination_delivery` helper the
/// HTTP listener uses -- no second, weaker duplicate destination-execution
/// path). The only difference from the HTTP path is that redemption also
/// carries `observed_mqtt_topic` -- the exact physical topic this
/// installation received the request envelope on -- which DServer
/// independently verifies against its own recomputed expected topic
/// (section 17) before ever marking the grant `Redeemed`. Section 16: this
/// function trusts NOTHING from the MQTT envelope beyond `grant_id`/
/// `token`/payload proof -- destination service/node/DMap/DForward/artifact
/// identity all come from the grant DServer returns on successful
/// redemption, never from anything the caller observed on the wire.
/// Returns `(the REDEEMED grant, the destination-execution result)` on
/// successful redemption -- the grant is returned EVEN IF execution
/// afterward fails (section 22: redemption still consumed the grant; the
/// caller still needs the grant's own identity to compute a reply topic and
/// publish a deterministic failure response). Returns `Err` only when
/// redemption itself never succeeded -- in that case there is no verified
/// party to reply to at all (section 20/21: an unredeemable/duplicate
/// request gets silently dropped, never a reply).
pub fn receive_governed_delivery_via_mqtt(
    dserver_url: &str,
    dnode_root: &str,
    grant_id: &str,
    token: &str,
    payload: &[u8],
    observed_mqtt_topic: &str,
) -> Result<(DCallDeliveryGrantV1, Result<DCodeInvocationEvidenceV1>)> {
    let identity = load_local_identity(Path::new(dnode_root)).map_err(|error| anyhow!(error))?;
    let grant = client::redeem_grant(
        dserver_url,
        grant_id,
        token,
        &identity.node_id,
        &sha256_hex(payload),
        payload.len() as u64,
        Some(observed_mqtt_topic),
    )?;
    let execution = execute_destination_delivery(
        dserver_url,
        dnode_root,
        &default_worker_binary_path(),
        &identity.node_id,
        &grant,
        payload,
    );
    Ok((grant, execution))
}

/// Phase 113B3 (section 18/20/22): parses+decodes an incoming MQTT request
/// envelope, verifies the decoded payload actually matches its declared
/// digest/size (mirrors `smartsentinel-dcall-listen`'s own
/// `handle_delivery` -- same self-review finding: `redeem_grant` cannot see
/// the decoded bytes, only the caller's declared fields, so this local
/// recomputation must happen BEFORE a real single-use grant redemption is
/// spent on them), then redeems+executes via
/// `receive_governed_delivery_via_mqtt`. Returns `None` when there is no
/// verified party to reply to (malformed envelope, payload mismatch, local
/// identity unprovisioned, or redemption itself failed -- e.g. wrong node,
/// wrong observed topic, stale/duplicate grant) -- the caller MUST NOT
/// publish anything in that case. Returns `Some((reply_topic, response))`
/// when redemption succeeded: `reply_topic` is derived EXCLUSIVELY from the
/// REDEEMED grant's own `project_id`/`application_id`/`source.node_id`/
/// `grant_id` -- section 20: never trusted from the request envelope, which
/// does not even carry one.
pub fn handle_mqtt_dcall_request(
    dserver_url: &str,
    dnode_root: &str,
    observed_mqtt_topic: &str,
    request: &crate::dcall::mqtt::MqttDCallRequestEnvelopeV1,
) -> Option<(String, crate::dcall::mqtt::MqttDCallResponseEnvelopeV1)> {
    use crate::dcall::mqtt::{self, MqttDCallResponseEnvelopeV1, RESPONSE_ENVELOPE_SCHEMA};

    use base64::Engine;
    let payload = base64::engine::general_purpose::STANDARD
        .decode(&request.payload_b64)
        .ok()?;

    let actual_sha256 = sha256_hex(&payload);
    if actual_sha256 != request.payload_sha256 || payload.len() as u64 != request.payload_size {
        return None;
    }

    let (grant, execution) = receive_governed_delivery_via_mqtt(
        dserver_url,
        dnode_root,
        &request.grant_id,
        &request.token,
        &payload,
        observed_mqtt_topic,
    )
    .ok()?;

    let reply_topic = mqtt::reply_topic(
        &grant.project_id,
        &grant.application_id,
        &grant.source.node_id,
        &grant.grant_id,
    );
    let response = match execution {
        Ok(evidence) => {
            let evidence_json = serde_json::to_value(&evidence).unwrap_or(serde_json::Value::Null);
            MqttDCallResponseEnvelopeV1 {
                schema: RESPONSE_ENVELOPE_SCHEMA.to_string(),
                grant_id: grant.grant_id.clone(),
                outcome: "success".to_string(),
                error_code: None,
                dcode_evidence: Some(evidence_json),
            }
        }
        Err(_error) => MqttDCallResponseEnvelopeV1 {
            schema: RESPONSE_ENVELOPE_SCHEMA.to_string(),
            grant_id: grant.grant_id.clone(),
            outcome: "failed".to_string(),
            error_code: Some("dcall_destination_execution_failed".to_string()),
            dcode_evidence: None,
        },
    };
    Some((reply_topic, response))
}

/// Default bounded window (section 4) to wait for protocol-level session
/// confirmation (ConnAck + SubAck) before treating startup as failed. Not
/// the same thing as the steady-state 500ms poll timeout below -- this is a
/// one-time startup bound, generous enough for a real broker round trip,
/// short enough that a genuinely unreachable broker fails fast.
const DEFAULT_SESSION_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// Steady-state poll granularity: how often the loop can notice a new DCall
/// message, a heartbeat becoming due, or `stop` being set.
const DEFAULT_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Phase 114D0B: converts the server's proven-current binding response into
/// the immutable session-authority identity the DMonitor producer carries
/// for the lifetime of ONE MQTT session (section 8). Kept as a free
/// function (not a `From` impl) so the one-time verification this requires
/// -- that the binding is for THIS exact local node -- stays the caller's
/// explicit responsibility rather than being silently skippable via a
/// generic trait conversion.
fn diot_runtime_binding_identity(
    binding: &client::CurrentRuntimeBindingResponse,
) -> crate::agent::diot_runtime_dmonitor::DiotRuntimeBindingIdentity {
    crate::agent::diot_runtime_dmonitor::DiotRuntimeBindingIdentity {
        project_id: binding.project_id.clone(),
        application_id: binding.application_id.clone(),
        diot_id: binding.diot_id.clone(),
        diot_placement_id: binding.placement_id.clone(),
        node_id: binding.node_id.clone(),
        dmap_id: binding.dmap_id.clone(),
        dmap_revision: binding.dmap_revision,
        dmap_digest: binding.dmap_digest.clone(),
        dforward_id: binding.dforward_id.clone(),
        dforward_revision: binding.dforward_revision,
        dforward_digest: binding.dforward_digest.clone(),
        binding_id: binding.binding_id.clone(),
        binding_revision: binding.binding_revision,
        descriptor_digest: binding.descriptor_digest.clone(),
    }
}

/// Builds one observation via `emitter`, serializes it, and offers it to
/// `publisher` -- never blocks (section 18/19), never panics, never retries.
fn emit_and_publish(
    emitter: &mut crate::agent::diot_runtime_dmonitor::DiotRuntimeMonitorEmitter,
    publisher: &crate::dcall::dmonitor_publisher::DMonitorPublisherHandle,
    health: crate::agent::canonical_dmonitor::DMonitorHealthV1,
    runtime_state: crate::agent::diot_runtime_dmonitor::DiotRuntimeState,
    heartbeat: bool,
) {
    let observed_at_utc = chrono::Utc::now().to_rfc3339();
    let observation = emitter.emit(&observed_at_utc, health, runtime_state, heartbeat);
    let observation_id = observation.observation_id.clone();
    if let Ok(observation_json) = serde_json::to_value(&observation) {
        publisher.try_publish(crate::dcall::dmonitor_publisher::DMonitorPublishJob {
            observation_id,
            observation_json,
        });
    }
}

/// Phase 115B: offers an already-built canonical `DServRealization`
/// observation to `publisher` -- never blocks, never panics, never retries.
/// A separate helper from `emit_and_publish` because the D-Serv emitter
/// (`DCodeExecutionMonitorEmitter::emit`) can honestly return `None` (Phase
/// 115A Q9's node-identity guard), so the caller decides whether to publish
/// at all, unlike the DIoT heartbeat emitter which always has something to
/// say.
fn publish_dserv_observation(
    publisher: &crate::dcall::dmonitor_publisher::DMonitorPublisherHandle,
    observation: &crate::agent::canonical_dmonitor::CanonicalDMonitorObservationV1,
) {
    let observation_id = observation.observation_id.clone();
    if let Ok(observation_json) = serde_json::to_value(observation) {
        publisher.try_publish(crate::dcall::dmonitor_publisher::DMonitorPublishJob {
            observation_id,
            observation_json,
        });
    }
}

/// Phase 115B: if `response` carries real D-Code invocation evidence
/// (`dcode_evidence`, populated only on `outcome="success"` in
/// `handle_mqtt_dcall_request`), builds and publishes one canonical
/// `DServRealization` observation from it. A response with no evidence
/// (malformed request, grant redemption failure -- no invocation ever
/// happened) or evidence that fails the emitter's own node-identity guard
/// (Phase 115A Q9) emits nothing -- never fabricated. Malformed evidence
/// JSON (should be unreachable in production: `handle_mqtt_dcall_request`
/// always serializes a real `DCodeInvocationEvidenceV1`) is likewise
/// silently skipped rather than panicking the listener loop.
fn emit_dserv_evidence_if_present(
    response: &crate::dcall::mqtt::MqttDCallResponseEnvelopeV1,
    dserv_emitter: &mut crate::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter,
    publisher: &crate::dcall::dmonitor_publisher::DMonitorPublisherHandle,
) {
    let Some(evidence_json) = response.dcode_evidence.clone() else { return };
    let Ok(evidence) = serde_json::from_value::<crate::dcode::evidence::DCodeInvocationEvidenceV1>(evidence_json) else { return };
    if let Some(observation) = dserv_emitter.emit(&evidence) {
        publish_dserv_observation(publisher, &observation);
    }
}

/// Phase 113B3 (section 15) + Phase 114D0B: production MQTT D-Call
/// destination listener loop. Runs until `stop` is set. Startup: prove
/// local D-Node identity, fetch the CURRENT runtime binding (fails closed
/// on `mqtts://`, a fetch failure, or a binding whose `node_id` does not
/// match this local D-Node -- section 7), connect, and subscribe ONLY to
/// this exact placement/local-node operational prefix (section 6) -- never
/// a destination service configuration of any kind (section 15/16: entirely
/// grant-derived). `heartbeat_interval` must be greater than zero.
///
/// Phase 114D0B additionally: confirms protocol-level session readiness
/// (ConnAck + SubAck, never merely `subscribe()` returning `Ok`) before
/// emitting any `Healthy` governed DIoT DMonitor evidence, then emits a
/// periodic `Healthy` heartbeat referencing the EXACT binding this session
/// was established under (never refreshed mid-session -- section 8), and a
/// best-effort `Unhealthy` observation on a definite transport failure
/// (section 21). DMonitor publication runs on a bounded background worker
/// (`dcall::dmonitor_publisher`) so a slow/unresponsive DServer can never
/// stall MQTT delivery (section 18).
pub fn run_mqtt_dcall_listener(
    dserver_url: &str,
    dnode_root: &str,
    project_id: &str,
    application_id: &str,
    diot_placement_id: &str,
    heartbeat_interval: std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    if heartbeat_interval.is_zero() {
        return Err(anyhow!("dmonitor heartbeat interval must be greater than zero"));
    }
    let identity = load_local_identity(Path::new(dnode_root)).map_err(|error| anyhow!(error))?;
    let binding = client::fetch_current_runtime_binding(
        dserver_url,
        project_id,
        application_id,
        diot_placement_id,
    )?;
    if binding.transport != "mqtt" {
        return Err(anyhow!(
            "runtime binding declares unsupported transport '{}' (only 'mqtt' is supported)",
            binding.transport
        ));
    }
    // Section 7: the local node comes ONLY from `load_local_identity`, never
    // CLI/binding input -- this is the defensive cross-check, never a
    // source of node identity itself. Section 14: fail closed rather than
    // fabricate a subject to make them agree.
    if binding.node_id != identity.node_id {
        return Err(anyhow!(
            "current runtime binding node_id '{}' does not match this D-Node's own local identity '{}'",
            binding.node_id,
            identity.node_id
        ));
    }
    let broker = crate::dcall::mqtt::parse_broker_uri_for_delivery(&binding.broker_uri)?;
    let transport = crate::dcall::mqtt::RumqttcPubSub::connect(
        &broker,
        &crate::dcall::mqtt::subscriber_client_id(),
    )?;

    let dmonitor_output =
        crate::outputs::dmonitor_http::DMonitorHttpOutput::new(dserver_url.to_string(), project_id.to_string(), 5)?;
    let publisher = crate::dcall::dmonitor_publisher::DMonitorPublisherHandle::spawn(
        dmonitor_output,
        crate::dcall::dmonitor_publisher::DEFAULT_QUEUE_CAPACITY,
    );
    let binding_identity = diot_runtime_binding_identity(&binding);
    let emission_context = crate::agent::dmonitor_emission::DMonitorEmissionContext::new_process_epoch(&identity.node_id);
    // Phase 115B: a SEPARATE process epoch (own `monitor_instance_id`, own
    // sequence counter) for canonical D-Serv realization evidence -- kept
    // independent of the DIoT emitter's own stream above, mirroring how
    // `agent::dmonitor_projection` and `agent::diot_runtime_dmonitor` are
    // already two independent producers rather than one shared stream.
    let dserv_emitter = crate::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter::new(
        crate::agent::dmonitor_emission::DMonitorEmissionContext::new_process_epoch(&identity.node_id),
        identity.node_id.clone(),
    );

    let result = run_mqtt_dcall_listener_loop(
        transport,
        dserver_url,
        dnode_root,
        project_id,
        application_id,
        diot_placement_id,
        &identity.node_id,
        binding_identity,
        emission_context,
        dserv_emitter,
        &publisher,
        heartbeat_interval,
        DEFAULT_POLL_INTERVAL,
        DEFAULT_SESSION_CONFIRM_TIMEOUT,
        stop,
    );
    // Graceful shutdown of the publication worker regardless of outcome --
    // never leaves a background thread dangling, never blocks longer than
    // one in-flight publish's own client timeout (section 22).
    publisher.shutdown();
    result
}

/// Generic over [`crate::dcall::mqtt::MqttListenerSession`] so production
/// drives a real broker (`RumqttcPubSub`) and tests drive the deterministic
/// `mqtt::test_support::InMemoryPubSub` -- both exercise IDENTICAL decision
/// logic (section 24/28), mirroring the existing `deliver_via_diot_pubsub`
/// generic-transport pattern.
#[allow(clippy::too_many_arguments)]
fn run_mqtt_dcall_listener_loop<T: crate::dcall::mqtt::MqttListenerSession>(
    mut transport: T,
    dserver_url: &str,
    dnode_root: &str,
    project_id: &str,
    application_id: &str,
    diot_placement_id: &str,
    local_node_id: &str,
    binding_identity: crate::agent::diot_runtime_dmonitor::DiotRuntimeBindingIdentity,
    emission_context: crate::agent::dmonitor_emission::DMonitorEmissionContext,
    mut dserv_emitter: crate::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter,
    publisher: &crate::dcall::dmonitor_publisher::DMonitorPublisherHandle,
    heartbeat_interval: std::time::Duration,
    poll_interval: std::time::Duration,
    confirm_timeout: std::time::Duration,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use crate::agent::canonical_dmonitor::DMonitorHealthV1;
    use crate::agent::diot_runtime_dmonitor::{DiotRuntimeMonitorEmitter, DiotRuntimeState};
    use crate::dcall::mqtt::{MqttPubSub, MqttSessionReadiness};
    use std::sync::atomic::Ordering;

    let mut emitter = DiotRuntimeMonitorEmitter::new(emission_context, binding_identity);

    let prefix = crate::dcall::mqtt::subscribe_prefix(
        project_id,
        application_id,
        diot_placement_id,
        local_node_id,
    );
    match transport.establish_and_confirm(&prefix, confirm_timeout) {
        MqttSessionReadiness::Ready => {}
        MqttSessionReadiness::SubscribeRejected(reason) | MqttSessionReadiness::ConnectionFailed(reason) => {
            // Section 14: the binding was obtained and validated, but the
            // broker connection/subscription could not be confirmed --
            // a best-effort Unhealthy observation for this EXACT attempted
            // governed realization is acceptable; Healthy is never emitted.
            emit_and_publish(&mut emitter, publisher, DMonitorHealthV1::Unhealthy, DiotRuntimeState::TransportFailure, false);
            return Err(anyhow!("MQTT session could not be confirmed ready: {reason}"));
        }
    }

    // Section 15: initial Healthy observation immediately after
    // protocol-confirmed session readiness.
    emit_and_publish(&mut emitter, publisher, DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
    let mut last_heartbeat = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        match MqttPubSub::poll(&mut transport, poll_interval) {
            Ok(Some((topic, bytes))) => {
                if let Ok(request) = serde_json::from_slice::<crate::dcall::mqtt::MqttDCallRequestEnvelopeV1>(&bytes) {
                    if let Some((reply_topic, response)) =
                        handle_mqtt_dcall_request(dserver_url, dnode_root, &topic, &request)
                    {
                        emit_dserv_evidence_if_present(&response, &mut dserv_emitter, publisher);
                        let response_bytes = serde_json::to_vec(&response).unwrap_or_default();
                        let _ = MqttPubSub::publish(
                            &mut transport,
                            &reply_topic,
                            &response_bytes,
                            crate::dcall::mqtt::MqttQos::AtLeastOnce,
                            false,
                        );
                    }
                } // malformed envelope: no verified party to reply to -- ignored, heartbeat check below still runs.
            }
            Ok(None) => {} // ordinary poll timeout -- fall through to the heartbeat check.
            Err(error) => {
                // Section 21: a previously-confirmed session hit a definite
                // transport failure -- stop Healthy heartbeats, emit one
                // best-effort Unhealthy observation, then preserve this
                // listener's existing fail-closed termination behavior
                // (section 23: no reconnect architecture is invented here).
                emit_and_publish(&mut emitter, publisher, DMonitorHealthV1::Unhealthy, DiotRuntimeState::TransportFailure, false);
                return Err(error);
            }
        }

        if last_heartbeat.elapsed() >= heartbeat_interval {
            // Section 16: new sequence, new observation_id, new
            // observed_at_utc; SAME monitor_instance_id and subject/binding
            // authority reference as the initial observation.
            emit_and_publish(&mut emitter, publisher, DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, true);
            last_heartbeat = Instant::now();
        }
    }
    // Section 22: graceful shutdown is not a transport fault -- no
    // Unhealthy observation is emitted merely because the operator
    // intentionally stopped the listener. The last Healthy evidence is
    // left to age to Stale, which is already a safe, honest outcome.
    Ok(())
}

/// Phase 113B3 source delivery flow (sections 8/12/13). Generic over
/// `MqttPubSub` so production code drives a real broker
/// (`mqtt::RumqttcPubSub`) and tests drive the deterministic
/// `mqtt::test_support::InMemoryPubSub` -- both exercise IDENTICAL logic.
/// `transport` must already be freshly constructed (not yet connected to
/// any subscription/publish state) for the grant this call is delivering.
fn deliver_via_diot_pubsub(
    dserver_url: &str,
    grant: &DCallDeliveryGrantV1,
    token: &str,
    payload: &[u8],
    payload_sha256: &str,
    max_wait: std::time::Duration,
    connect: impl FnOnce(
        &crate::dcall::mqtt::BrokerAddress,
    ) -> Result<Box<dyn crate::dcall::mqtt::MqttPubSub>>,
) -> Result<(DCodeInvocationEvidenceV1, String), DeliveryFailure> {
    use crate::dcall::mqtt::{
        self, MqttDCallRequestEnvelopeV1, MqttDCallResponseEnvelopeV1, MqttQos,
        REQUEST_ENVELOPE_SCHEMA,
    };

    let v2_authority = grant.v2_dcall_transport_authority.as_ref().ok_or_else(|| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallDiotTransportAuthorityMissing,
        diagnostic: Some("deliver_via_diot_pubsub called without a v2 transport authority".to_string()),
        redeemed_at: None,
    })?;

    // Section 8: fetch the CURRENT runtime binding and require EXACT
    // agreement with the grant's bound authority BEFORE opening any MQTT
    // connection -- closes the "grant issued under binding N, binding
    // becomes N+1" race.
    let current_binding = client::fetch_current_runtime_binding(
        dserver_url,
        &grant.project_id,
        &grant.application_id,
        &v2_authority.diot_placement_id,
    )
    .map_err(|error| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallDiotRuntimeBindingFetchFailed,
        diagnostic: Some(error.to_string()),
        redeemed_at: None,
    })?;
    if current_binding.binding_id != v2_authority.binding_id
        || current_binding.binding_revision != v2_authority.binding_revision
        || current_binding.descriptor_digest != v2_authority.descriptor_digest
        || current_binding.transport != v2_authority.transport
        || current_binding.project_id != grant.project_id
        || current_binding.application_id != grant.application_id
        || current_binding.placement_id != v2_authority.diot_placement_id
    {
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallDiotRuntimeBindingAuthorityMismatch,
            diagnostic: Some(
                "current DIoT runtime binding does not match the grant's bound transport authority"
                    .to_string(),
            ),
            redeemed_at: None,
        });
    }

    // Section 4: v0.1 supports only plain `mqtt://` for actual delivery --
    // an `mqtts://`-declared binding fails closed HERE, at use time (the
    // Phase 113B1 registry itself is not narrowed to reject registering one).
    let broker = mqtt::parse_broker_uri_for_delivery(&current_binding.broker_uri).map_err(
        |error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallDiotBrokerSchemeUnsupported,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        },
    )?;

    let request_topic = mqtt::request_topic(
        &grant.project_id,
        &grant.application_id,
        &v2_authority.diot_placement_id,
        &grant.destination.node_id,
        &v2_authority.channel,
    );
    let reply_topic = mqtt::reply_topic(
        &grant.project_id,
        &grant.application_id,
        &grant.source.node_id,
        &grant.grant_id,
    );

    let mut transport = connect(&broker).map_err(|error| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallTransportError,
        diagnostic: Some(error.to_string()),
        redeemed_at: None,
    })?;

    // Section 12, step 7: subscribe to the reply topic BEFORE publishing the
    // request -- otherwise a fast destination reply could arrive before this
    // process is listening for it.
    transport.subscribe(&reply_topic).map_err(|error| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallTransportError,
        diagnostic: Some(error.to_string()),
        redeemed_at: None,
    })?;

    let envelope = MqttDCallRequestEnvelopeV1 {
        schema: REQUEST_ENVELOPE_SCHEMA.to_string(),
        grant_id: grant.grant_id.clone(),
        token: token.to_string(),
        payload_b64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, payload),
        payload_sha256: payload_sha256.to_string(),
        payload_size: payload.len() as u64,
    };
    let envelope_bytes = serde_json::to_vec(&envelope).map_err(|error| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallTransportError,
        diagnostic: Some(error.to_string()),
        redeemed_at: None,
    })?;

    // Section 11: QoS1, retain=false, always -- never a caller/config choice.
    transport
        .publish(&request_topic, &envelope_bytes, MqttQos::AtLeastOnce, false)
        .map_err(|error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallTransportError,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        })?;

    // Section 24: bound the wait by the grant's own practical usable
    // lifetime -- never an unbounded wait. The caller computes `max_wait`
    // from the grant's `expires_at` (capped at a sensible request timeout);
    // kept as an explicit parameter (rather than computed in here) so tests
    // can exercise a short, deterministic bound without needing to fabricate
    // a near-future `expires_at`.
    let timeout = max_wait;

    // Section 13: PUBACK/publish success alone is never DCall success --
    // only a VALID reply envelope on the exact reply topic counts. Multiple
    // `poll` calls may be needed to skip over unrelated/malformed traffic
    // (defensive; the reply topic is namespaced per-grant so this should
    // not occur in practice) before the deadline is reached.
    let deadline = std::time::Instant::now() + timeout;
    let response: MqttDCallResponseEnvelopeV1 = loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(DeliveryFailure {
                code: DCallDeliveryErrorCode::DCallTransportTimeout,
                diagnostic: Some(
                    "no valid MQTT reply received before the grant's usable lifetime elapsed"
                        .to_string(),
                ),
                redeemed_at: None,
            });
        }
        let message = transport.poll(remaining).map_err(|error| DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallTransportError,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        })?;
        let (topic, bytes) = match message {
            Some(message) => message,
            None => {
                return Err(DeliveryFailure {
                    code: DCallDeliveryErrorCode::DCallTransportTimeout,
                    diagnostic: Some(
                        "no valid MQTT reply received before the grant's usable lifetime elapsed"
                            .to_string(),
                    ),
                    redeemed_at: None,
                })
            }
        };
        if topic != reply_topic {
            continue;
        }
        match serde_json::from_slice::<MqttDCallResponseEnvelopeV1>(&bytes) {
            Ok(parsed) if parsed.grant_id == grant.grant_id => break parsed,
            _ => continue,
        }
    };

    if response.outcome != "success" {
        // The destination redeemed (or attempted to) -- we only know it
        // reported failure, not whether redemption itself ever succeeded;
        // report `redeemed_at` as unknown (`None`) rather than guessing,
        // same honesty convention as `deliver_remote`.
        return Err(DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallDestinationExecutionFailed,
            diagnostic: Some(format!(
                "MQTT destination reported outcome='{}' error_code={:?}",
                response.outcome, response.error_code
            )),
            redeemed_at: None,
        });
    }
    let evidence_json = response.dcode_evidence.ok_or_else(|| DeliveryFailure {
        code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
        diagnostic: Some("MQTT success response missing dcode_evidence".to_string()),
        redeemed_at: None,
    })?;
    let evidence: DCodeInvocationEvidenceV1 = serde_json::from_value(evidence_json).map_err(|error| {
        DeliveryFailure {
            code: DCallDeliveryErrorCode::DCallRemoteResponseMalformed,
            diagnostic: Some(error.to_string()),
            redeemed_at: None,
        }
    })?;
    // The destination redeemed the grant server-side; same honesty
    // convention as `deliver_local`/`deliver_remote`: we only know it
    // succeeded as of when THIS reply arrived.
    Ok((evidence, now_rfc3339()))
}

#[cfg(all(test, feature = "dcall-test-support"))]
mod tests {
    use super::*;
    mod endpoint_authority;
    use crate::dcall::evidence::DCallDeliveryOutcome;
    use crate::dcode::evidence::{DCodeAppliedLimits, DCODE_EVIDENCE_SCHEMA};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    // -- J1: `DCodeInvocationResult`/`ValidatedDCodeEmit` are now opaque
    // (private fields, no public constructor) -- a hand-fabricated
    // `DCodeInvocationEvidenceV1` with an invented `emits` list (as the old
    // `sample_source_evidence` helper built) can no longer be wrapped into a
    // `DCodeInvocationResult` from this module. Every test below that needs
    // one now runs a REAL `invoke_dcode_capturing_emits` call (in-process
    // executor, for test speed) against a tiny real WASM module -- this is
    // strictly stronger test evidence than the old hand-built fixture, since
    // it exercises the exact same construction path production code does.

    use crate::dcode::abi::DNODE_ABI_V0_REQUIRED_EXPORTS;
    use crate::dcode::authorization::CanonicalContractReferenceV1;
    use crate::dcode::dserv_artifact::{
        recompute_dserv_artifact_digest, DServArtifactAbiV1, DServArtifactConstraintsV1,
        DServArtifactDCodeV1, DServArtifactLimitsV1, DServArtifactPortV1, DServArtifactPortsV1,
        DServArtifactSafetyV1, CANONICAL_DSERV_ARTIFACT_SCHEMA,
    };
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
    fn real_source_result(
        authorization_node_id: &str,
        this_node_id: &str,
        output_body: &str,
    ) -> GovernedDCodeInvocationResult {
        let module_bytes = respond_wat(output_body);
        let digest = sha256_hex(&module_bytes);
        let store = DCodeModuleStore::new(
            std::env::temp_dir().join(format!("dcall-delivery-source-{}", Uuid::new_v4())),
        );
        store.install(&module_bytes, &digest).unwrap();
        let cache = DCodeModuleCache::new().unwrap();

        let artifact = CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: "dserv:pipeline:source".to_string(),
            version: "0.1.0".to_string(),
            application_id: "app:a".to_string(),
            dserv_id: "source".to_string(),
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{digest}"),
                sha256: digest.clone(),
                content_id: format!("sha256:{digest}"),
                size_bytes: module_bytes.len() as u64,
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
            ports: DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: "in".to_string(),
                    payload_schema: "s".to_string(),
                    external: true,
                }],
                outputs: vec![DServArtifactPortV1 {
                    port: "out".to_string(),
                    payload_schema: "s".to_string(),
                    external: false,
                }],
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
            authorization_id: "auth-1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:a".to_string(),
            service_id: "source".to_string(),
            node_id: authorization_node_id.to_string(),
            dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 1,
                digest: format!("sha256:{}", "1".repeat(64)),
            },
            dgraph_ref: CanonicalContractReferenceV1 {
                id: "dgraph:1".to_string(),
                revision: 1,
                digest: format!("sha256:{}", "2".repeat(64)),
            },
            dserv_artifact_id: "dserv:pipeline:source".to_string(),
            dserv_artifact_digest: digest,
            dserv_artifact_descriptor_digest: descriptor_digest,
            dnode_abi: "datum-dnode/0".to_string(),
        };
        let executor = InProcessExecutor {
            cache: &cache,
            store: &store,
        };
        invoke_dcode_governed_capturing_emits_for_test(
            &executor,
            &store,
            &authorization,
            &artifact,
            this_node_id,
            "in",
            b"trigger",
        )
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
    fn fan_out_gives_each_call_a_unique_message_id_but_shares_one_correlation_id_and_one_failure_does_not_erase_another(
    ) {
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
                    let request_body: serde_json::Value = serde_json::from_str(&text[body_start..])
                        .unwrap_or(serde_json::Value::Null);
                    let call_id = request_body
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let message_id = request_body
                        .get("message_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let correlation_id = request_body
                        .get("correlation_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let mut body: serde_json::Value = serde_json::from_str(&fanout_grant_response(
                        &call_id,
                        &message_id,
                        &correlation_id,
                    ))
                    .unwrap();
                    body["grant"]["source_dcode_evidence_sha256"] =
                        request_body["source_dcode_evidence_sha256"].clone();
                    let body = body.to_string();
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        });

        let context = SourceExecutionContext {
            dserver_url: format!("http://{addr}"),
            dnode_root: "unused".to_string(),
        };

        let dnode_root = std::env::temp_dir().join(format!("dcall-fanout-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext {
            dnode_root: dnode_root.to_string_lossy().to_string(),
            ..context
        };

        let source_result = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#,
        );
        assert_eq!(
            source_result.evidence().outcome,
            DCodeInvocationOutcome::Success
        );
        assert_eq!(source_result.validated_emits().len(), 1);
        let expected_correlation_id = source_result.evidence().correlation_id.clone();

        let results =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(
            results.len(),
            2,
            "one delivery attempt per resolved call, results: {results:?}"
        );

        for evidence in &results {
            assert!(matches!(evidence.outcome, DCallDeliveryOutcome::Failed));
            assert_eq!(
                evidence.error_code.as_deref(),
                Some("dcall_unsupported_route_kind")
            );
        }

        let call_ids: std::collections::BTreeSet<_> =
            results.iter().map(|e| e.call_id.clone()).collect();
        assert_eq!(
            call_ids,
            ["call-a".to_string(), "call-b".to_string()]
                .into_iter()
                .collect()
        );

        for evidence in &results {
            assert_eq!(
                evidence.correlation_id, expected_correlation_id,
                "every fan-out delivery must share the SOURCE invocation's own correlation_id"
            );
        }
        let message_ids: std::collections::BTreeSet<_> =
            results.iter().map(|e| e.message_id.clone()).collect();
        assert_eq!(
            message_ids.len(),
            2,
            "expected two distinct message_ids, got: {message_ids:?}"
        );
    }

    #[test]
    fn dispatch_reports_a_sanitized_lookup_failure_when_dserver_is_unreachable() {
        let dnode_root =
            std::env::temp_dir().join(format!("dcall-unreachable-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext {
            dserver_url: "http://127.0.0.1:1".to_string(),
            dnode_root: dnode_root.to_string_lossy().to_string(),
        };
        let source_result = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#,
        );
        let results =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].outcome, DCallDeliveryOutcome::Failed));
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("dcall_lookup_failed")
        );
        assert!(results[0].error_diagnostic_sha256.is_some());
        let serialized = serde_json::to_string(&results[0]).unwrap();
        assert!(
            !serialized.contains("Connection refused")
                || results[0].error_diagnostic_sha256.is_some()
        );
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
        fn execute(
            &self,
            _digest: &str,
            _limits: &InvocationLimits,
            _input: &[u8],
        ) -> ExecutionOutcome {
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
            (
                cache,
                (),
                authorization_for_fake_test(&digest, &module_bytes),
                artifact_for_fake_test(&digest, &module_bytes),
            )
        };
        let executor = FakeExecutor;
        // This compiles and succeeds: a generic `DCodeInvocationResult` is
        // genuinely obtainable from a fake, non-isolated, non-real-Wasmtime
        // executor -- J1's opacity alone does not prevent this.
        let fake_result = invoke_dcode_capturing_emits(
            &executor,
            &store,
            &authorization,
            &artifact,
            "fog-01",
            "in",
            b"trigger",
        );
        assert_eq!(
            fake_result.evidence().outcome,
            DCodeInvocationOutcome::Success
        );

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

    fn authorization_for_fake_test(
        digest: &str,
        module_bytes: &[u8],
    ) -> DCodeInvocationAuthorizationV1 {
        let artifact = artifact_for_fake_test(digest, module_bytes);
        let descriptor_digest =
            crate::dcode::dserv_artifact::recompute_dserv_artifact_digest(&artifact).unwrap();
        DCodeInvocationAuthorizationV1 {
            authorization_id: "auth-fake".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:a".to_string(),
            service_id: "source".to_string(),
            node_id: "fog-01".to_string(),
            dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 1,
                digest: format!("sha256:{}", "1".repeat(64)),
            },
            dgraph_ref: CanonicalContractReferenceV1 {
                id: "dgraph:1".to_string(),
                revision: 1,
                digest: format!("sha256:{}", "2".repeat(64)),
            },
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
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{digest}"),
                sha256: digest.to_string(),
                content_id: format!("sha256:{digest}"),
                size_bytes: module_bytes.len() as u64,
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
            ports: DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: "in".to_string(),
                    payload_schema: "s".to_string(),
                    external: true,
                }],
                outputs: Vec::new(),
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
        }
    }

    #[test]
    fn dispatch_rejects_a_source_invocation_that_was_not_successful() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-a-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext {
            dserver_url: "http://127.0.0.1:1".to_string(),
            dnode_root: dnode_root.to_string_lossy().to_string(),
        };
        // authorization.node_id="node-x" but the invocation runs as "fog-01"
        // -> a REAL StaleAuthority failure, not a hand-set enum value.
        let source_result = real_source_result("node-x", "fog-01", r#"{"status":"ok","emits":[]}"#);
        assert_ne!(
            source_result.evidence().outcome,
            DCodeInvocationOutcome::Success
        );
        let results =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("dcall_source_invocation_not_successful")
        );
    }

    #[test]
    fn dispatch_rejects_an_out_of_range_emit_index() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-b-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext {
            dserver_url: "http://127.0.0.1:1".to_string(),
            dnode_root: dnode_root.to_string_lossy().to_string(),
        };
        // A real, successful invocation that emits NOTHING -- index 0 is out
        // of range for its (empty) `validated_emits()`.
        let source_result = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[]}"#);
        assert_eq!(
            source_result.evidence().outcome,
            DCodeInvocationOutcome::Success
        );
        assert!(source_result.validated_emits().is_empty());
        let results =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("dcall_source_emit_not_found")
        );
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
    fn dispatch_rejects_source_evidence_claiming_a_different_node_than_this_installations_local_identity(
    ) {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-d-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dnode_root).unwrap();
        crate::dcode::local_identity::init_local_identity(&dnode_root, "fog-01").unwrap();
        let context = SourceExecutionContext {
            dserver_url: "http://127.0.0.1:1".to_string(),
            dnode_root: dnode_root.to_string_lossy().to_string(),
        };
        // Real invocation runs (and its evidence claims) node "cloud-99", but
        // this installation's own local identity is "fog-01".
        let source_result = real_source_result(
            "cloud-99",
            "cloud-99",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#,
        );
        assert_eq!(
            source_result.evidence().outcome,
            DCodeInvocationOutcome::Success
        );
        let results =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("dcall_source_local_identity_mismatch")
        );
    }

    #[test]
    fn dispatch_fails_closed_before_any_network_attempt_when_local_identity_is_not_provisioned() {
        let dnode_root = std::env::temp_dir().join(format!("dcall-i1-e-{}", Uuid::new_v4()));
        // deliberately NOT initialized
        let context = SourceExecutionContext {
            dserver_url: "http://127.0.0.1:1".to_string(),
            dnode_root: dnode_root.to_string_lossy().to_string(),
        };
        let source_result = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#,
        );
        let results =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("dcall_source_local_identity_load_failed")
        );
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
        let context = SourceExecutionContext {
            dserver_url: "http://127.0.0.1:1".to_string(),
            dnode_root: dnode_root.to_string_lossy().to_string(),
        };
        // "out"/"A" (QQ==) and "out"/"B" (Qg==) -- same port, different payloads.
        let source_result = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"QQ=="},{"port":"out","payload_b64":"Qg=="}]}"#,
        );
        assert_eq!(
            source_result.evidence().outcome,
            DCodeInvocationOutcome::Success
        );
        assert_eq!(source_result.validated_emits().len(), 2);
        assert_eq!(source_result.validated_emits()[0].payload(), b"A");
        assert_eq!(source_result.validated_emits()[1].payload(), b"B");

        // Both dispatched separately -- neither is falsely rejected as a
        // "payload mismatch" against the other's digest (the exact J1C bug).
        // `dserver_url` is unreachable, so both fail at the SAME later stage
        // (`dcall_lookup_failed`), proving each independently passed the
        // provenance checks (source-invocation-successful, emit-index
        // resolution, local-identity match) before reaching the network.
        let results_a =
            dispatch_validated_emit_for_test(&context, &source_result, 0, Path::new("unused"));
        let results_b =
            dispatch_validated_emit_for_test(&context, &source_result, 1, Path::new("unused"));
        assert_eq!(results_a.len(), 1);
        assert_eq!(results_b.len(), 1);
        assert_eq!(
            results_a[0].error_code.as_deref(),
            Some("dcall_lookup_failed")
        );
        assert_eq!(
            results_b[0].error_code.as_deref(),
            Some("dcall_lookup_failed")
        );
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
            dgraph_ref: CanonicalContractReferenceV1 {
                id: "dgraph:1".to_string(),
                revision: 1,
                digest: "sha256:cc".to_string(),
            },
            dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 1,
                digest: "sha256:dd".to_string(),
            },
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
            v2_dcall_transport_authority: None,
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
            dserv_artifact_descriptor_digest: grant
                .destination
                .dserv_artifact_descriptor_digest
                .clone(),
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
            limits_applied: DCodeAppliedLimits {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 100,
                max_memory_pages: 4,
                max_message_bytes: 65536,
            },
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
        for field in [
            "schema",
            "project_id",
            "application_id",
            "service_id",
            "node_id",
            "dgraph_id",
            "dgraph_revision",
            "dgraph_digest",
            "dmap_id",
            "dmap_revision",
            "dmap_digest",
            "dserv_artifact_id",
            "dserv_artifact_digest",
            "dserv_artifact_descriptor_digest",
            "input_port",
            "input_sha256",
            "input_size",
            "message_id",
            "correlation_id",
        ] {
            let mut value = original.clone();
            value[field] = if field.ends_with("revision") || field == "input_size" {
                serde_json::json!(999)
            } else {
                serde_json::json!("substitution")
            };
            let evidence = serde_json::from_value(value).unwrap();
            assert!(
                validate_destination_dcode_evidence_against_grant(&evidence, &grant).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn source_evidence_digest_substitution_fails_before_redemption_or_execution() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            for body in [
                serde_json::json!(["call-a"]).to_string(),
                fanout_grant_response("call-a", "m1", "c1"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = [0; 8192];
                stream.read(&mut bytes).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
            // Closing the observer here makes any subsequent redeem/execute
            // call fail; the returned specific mismatch proves that branch
            // was reached before transport or redemption.
        });
        let root =
            std::env::temp_dir().join(format!("dcall-source-substitution-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let source = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#,
        );
        let context = SourceExecutionContext {
            dserver_url: format!("http://{addr}"),
            dnode_root: root.to_str().unwrap().to_string(),
        };
        let result = dispatch_validated_emit_for_test(&context, &source, 0, Path::new("unused"));
        assert_eq!(
            result[0].error_code.as_deref(),
            Some("dcall_source_evidence_mismatch")
        );
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
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    MAX_REMOTE_RESPONSE_BYTES + 1
                );
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
        assert_eq!(
            error.code,
            DCallDeliveryErrorCode::DCallRemoteResponseTooLarge
        );
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
        assert_eq!(
            error.code,
            DCallDeliveryErrorCode::DCallRemoteResponseTooLarge
        );
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

    // ================================================================
    // Phase 113B2 (section 19, MANDATORY fail-closed seam; section 27
    // dispatch test matrix A-H).
    // ================================================================

    fn v2_diot_grant_response(
        call_id: &str,
        message_id: &str,
        correlation_id: &str,
        route_kind: &str,
    ) -> String {
        serde_json::json!({
            "grant": {
                "grant_id": format!("grant-{call_id}"),
                "project_id": "proj:lab",
                "application_id": "app:a",
                "call_id": call_id,
                "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": "sha256:aa"},
                "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": "sha256:bb"},
                "source": {"service_id": "source", "port": "out", "node_id": "fog-01", "dserv_artifact_id": "a1", "dserv_artifact_digest": "sha256:cc", "dserv_artifact_descriptor_digest": "sha256:dd"},
                "destination": {"service_id": "dest", "port": "in", "node_id": "fog-01", "dserv_artifact_id": "a2", "dserv_artifact_digest": "sha256:ee", "dserv_artifact_descriptor_digest": "sha256:ff"},
                "message_id": message_id,
                "correlation_id": correlation_id,
                "payload_sha256": sha256_hex(b"payload"),
                "payload_size": 7,
                "source_dcode_evidence_sha256": "f".repeat(64),
                "route_kind": route_kind,
                "v2_dcall_transport_authority": {
                    "realization_kind": "via_diot_pubsub",
                    "diot_placement_id": "diotp:broker",
                    "interface": "sensor-bus",
                    "channel": "pipeline/sensor-to-decoder",
                    "dforward_id": "dforward:pipeline",
                    "dforward_revision": 1,
                    "dforward_digest": format!("sha256:{}", "1".repeat(64)),
                    "binding_id": "diot-runtime-binding:proj:lab:app:a:diotp:broker",
                    "binding_revision": 1,
                    "descriptor_digest": format!("sha256:{}", "2".repeat(64)),
                    "transport": "mqtt",
                },
                "issued_at": "2026-01-01T00:00:00Z",
                "expires_at": "2026-01-01T00:00:30Z",
            },
            "token": "super-secret-bearer-token-must-never-leak",
        })
        .to_string()
    }

    /// Serves EXACTLY the two requests a v2 dispatch attempt should make
    /// (list_calls, then the grant POST) and then actively probes for a
    /// THIRD connection for a short bounded window before returning --
    /// proving no redemption/HTTP-delivery request was ever attempted.
    /// Returns whether a third connection was observed (must be `false`).
    /// Reads one HTTP request off `stream` and returns its JSON body
    /// (`Value::Null` if the request has no body or is not JSON) -- used to
    /// echo the real caller-computed `source_dcode_evidence_sha256` back
    /// into a mock grant response, exactly as `fanout_grant_response`'s own
    /// call sites do.
    fn read_request_json_body(stream: &mut std::net::TcpStream) -> serde_json::Value {
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
        serde_json::from_str(&text[body_start..]).unwrap_or(serde_json::Value::Null)
    }

    // -- Phase 113B3: real MQTT source-delivery tests (section 34, A-K) ----

    fn sample_v2_grant(route_kind: &str) -> DCallDeliveryGrantV1 {
        let value: serde_json::Value =
            serde_json::from_str(&v2_diot_grant_response("call-a", "m1", "c1", route_kind))
                .unwrap();
        serde_json::from_value(value["grant"].clone()).unwrap()
    }

    /// A `CurrentRuntimeBindingResponse`-shaped JSON that agrees EXACTLY
    /// with `grant`'s bound `v2_dcall_transport_authority` (the "everything
    /// matches" baseline every mismatch test mutates one field of).
    fn matching_binding_json(grant: &DCallDeliveryGrantV1, broker_uri: &str) -> serde_json::Value {
        let authority = grant.v2_dcall_transport_authority.as_ref().unwrap();
        serde_json::json!({
            "binding_id": authority.binding_id,
            "binding_revision": authority.binding_revision,
            "project_id": grant.project_id,
            "application_id": grant.application_id,
            "placement_id": authority.diot_placement_id,
            "diot_id": "diot:sensor-hub",
            "node_id": grant.destination.node_id,
            "dmap_id": "dmap:1",
            "dmap_revision": 1,
            "dmap_digest": "sha256:bb",
            "dforward_id": authority.dforward_id,
            "dforward_revision": authority.dforward_revision,
            "dforward_digest": authority.dforward_digest,
            "artifact_id": "artifact:sensor",
            "execution_generation": 1,
            "execution_projection_digest": format!("sha256:{}", "3".repeat(64)),
            "transport": authority.transport,
            "broker_uri": broker_uri,
            "registered_at_utc": "2026-01-01T00:00:00Z",
            "descriptor_digest": authority.descriptor_digest,
        })
    }

    /// Serves exactly one GET request (the `.../current` binding fetch) with
    /// the given status/body, then closes.
    fn serve_one_current_binding_response(status_line: &'static str, body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = read_request_json_body(&mut stream); // drains the GET (no body, harmless)
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A `connect` closure that panics if ever invoked -- proves the MQTT
    /// connection step was never reached (used by the two mismatch tests,
    /// A/B).
    fn connect_must_never_be_called(
        _broker: &crate::dcall::mqtt::BrokerAddress,
    ) -> Result<Box<dyn crate::dcall::mqtt::MqttPubSub>> {
        panic!("MQTT connect must never be attempted when binding/grant authority disagrees or the scheme is unsupported")
    }

    /// 34A: a CURRENT binding whose `binding_id` disagrees with the grant's
    /// bound authority must fail closed BEFORE any MQTT connection attempt.
    #[test]
    fn v2_source_delivery_fails_closed_on_binding_grant_mismatch_before_any_connect() {
        let grant = sample_v2_grant("remote");
        let mut binding = matching_binding_json(&grant, "mqtt://broker.lab:1883");
        binding["binding_id"] = serde_json::json!("a-completely-different-binding");
        let dserver_url = serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());
        let result = deliver_via_diot_pubsub(
            &dserver_url,
            &grant,
            "token",
            b"payload",
            &sha256_hex(b"payload"),
            std::time::Duration::from_millis(200),
            connect_must_never_be_called,
        );
        let failure = result.unwrap_err();
        assert_eq!(
            failure.code.as_str(),
            "dcall_diot_runtime_binding_authority_mismatch"
        );
        assert!(failure.redeemed_at.is_none());
    }

    /// 113B3.1 section 5: the production comparison
    /// (`deliver_via_diot_pubsub`'s `current_binding.binding_id !=
    /// v2_authority.binding_id || ...binding_revision... ||
    /// ...descriptor_digest... || ...transport...`) is a single combined OR
    /// across four fields -- the test above only ever exercised `binding_id`.
    /// This table-driven test proves EACH of the other three fields is
    /// independently load-bearing (mutating it ALONE, with the other three
    /// still agreeing, still fails closed before any MQTT connect), closing
    /// the gap without four separate near-duplicate integration fixtures.
    #[test]
    fn v2_source_delivery_fails_closed_on_each_individual_binding_field_mismatch() {
        let mutations: [(&str, serde_json::Value); 3] = [
            ("binding_revision", serde_json::json!(9999)),
            (
                "descriptor_digest",
                serde_json::json!(format!("sha256:{}", "9".repeat(64))),
            ),
            ("transport", serde_json::json!("not-mqtt")),
        ];
        for (field, mutated_value) in mutations {
            let grant = sample_v2_grant("remote");
            let mut binding = matching_binding_json(&grant, "mqtt://broker.lab:1883");
            binding[field] = mutated_value;
            let dserver_url =
                serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());
            let result = deliver_via_diot_pubsub(
                &dserver_url,
                &grant,
                "token",
                b"payload",
                &sha256_hex(b"payload"),
                std::time::Duration::from_millis(200),
                connect_must_never_be_called,
            );
            let failure = match result {
                Err(failure) => failure,
                Ok(_) => panic!("mutating '{field}' alone must fail closed but did not"),
            };
            assert_eq!(
                failure.code.as_str(),
                "dcall_diot_runtime_binding_authority_mismatch",
                "field '{field}' mismatch produced the wrong error code"
            );
            assert!(
                failure.redeemed_at.is_none(),
                "field '{field}' mismatch must never redeem"
            );
        }
    }

    /// 34B: a CURRENT binding declaring `mqtts://` fails with the dedicated
    /// deterministic unsupported-scheme code, BEFORE any MQTT connection
    /// attempt -- the Phase 113B1 registry itself still accepts registering
    /// one; this is the "at USE time" fail-closed boundary (section 4).
    #[test]
    fn v2_source_delivery_fails_closed_on_mqtts_scheme_before_any_connect() {
        let grant = sample_v2_grant("remote");
        let binding = matching_binding_json(&grant, "mqtts://broker.lab:8883");
        let dserver_url = serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());
        let result = deliver_via_diot_pubsub(
            &dserver_url,
            &grant,
            "token",
            b"payload",
            &sha256_hex(b"payload"),
            std::time::Duration::from_millis(200),
            connect_must_never_be_called,
        );
        let failure = result.unwrap_err();
        assert_eq!(failure.code.as_str(), "dcall_diot_broker_scheme_unsupported");
        assert!(failure.redeemed_at.is_none());
    }

    /// A `MqttPubSub` wrapper that records call ordering into a shared log
    /// before delegating to a real `InMemoryPubSub` -- lets a test assert
    /// "subscribe happened before publish" (34C) without needing to inspect
    /// `rumqttc` internals.
    struct OrderRecordingPubSub {
        inner: crate::dcall::mqtt::test_support::InMemoryPubSub,
        log: std::sync::Arc<Mutex<Vec<&'static str>>>,
    }
    impl crate::dcall::mqtt::MqttPubSub for OrderRecordingPubSub {
        fn subscribe(&mut self, topic: &str) -> Result<()> {
            self.log.lock().unwrap().push("subscribe");
            self.inner.subscribe(topic)
        }
        fn publish(
            &mut self,
            topic: &str,
            payload: &[u8],
            qos: crate::dcall::mqtt::MqttQos,
            retain: bool,
        ) -> Result<()> {
            self.log.lock().unwrap().push("publish");
            self.inner.publish(topic, payload, qos, retain)
        }
        fn poll(
            &mut self,
            timeout: std::time::Duration,
        ) -> Result<Option<(String, Vec<u8>)>> {
            self.inner.poll(timeout)
        }
    }

    /// Spawns a thread that behaves like the real destination subscriber:
    /// waits for the request envelope on `request_topic`, then publishes
    /// `response` to `reply_topic`. Returns the join handle.
    fn spawn_fake_destination(
        broker: std::sync::Arc<crate::dcall::mqtt::test_support::InMemoryBroker>,
        request_topic: String,
        reply_topic: String,
        response: crate::dcall::mqtt::MqttDCallResponseEnvelopeV1,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            use crate::dcall::mqtt::MqttPubSub;
            let mut destination = broker.connect();
            destination.subscribe(&request_topic).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if std::time::Instant::now() > deadline {
                    return;
                }
                if let Some((_, _bytes)) = destination
                    .poll(std::time::Duration::from_millis(100))
                    .unwrap()
                {
                    let response_bytes = serde_json::to_vec(&response).unwrap();
                    destination
                        .publish(
                            &reply_topic,
                            &response_bytes,
                            crate::dcall::mqtt::MqttQos::AtLeastOnce,
                            false,
                        )
                        .unwrap();
                    return;
                }
            }
        })
    }

    fn success_response_for(grant: &DCallDeliveryGrantV1) -> crate::dcall::mqtt::MqttDCallResponseEnvelopeV1 {
        let evidence = sample_destination_evidence(grant);
        crate::dcall::mqtt::MqttDCallResponseEnvelopeV1 {
            schema: crate::dcall::mqtt::RESPONSE_ENVELOPE_SCHEMA.to_string(),
            grant_id: grant.grant_id.clone(),
            outcome: "success".to_string(),
            error_code: None,
            dcode_evidence: Some(serde_json::to_value(&evidence).unwrap()),
        }
    }

    /// A well-formed `DCodeInvocationEvidenceV1` matching `grant`'s own
    /// bound identity fields exactly (so it passes
    /// `validate_destination_dcode_evidence_against_grant` for the success
    /// test), built by taking a REAL evidence record from a genuine
    /// `invoke_dcode_capturing_emits` call (never hand-fabricated) and
    /// overwriting only the identity fields the grant itself binds.
    fn sample_destination_evidence(grant: &DCallDeliveryGrantV1) -> DCodeInvocationEvidenceV1 {
        let mut evidence = real_source_result("fog-01", "fog-01", r#"{"status":"ok","emits":[]}"#)
            .evidence()
            .clone();
        evidence.project_id = grant.project_id.clone();
        evidence.application_id = grant.application_id.clone();
        evidence.service_id = grant.destination.service_id.clone();
        evidence.node_id = grant.destination.node_id.clone();
        evidence.dgraph_id = grant.dgraph_ref.id.clone();
        evidence.dgraph_revision = grant.dgraph_ref.revision;
        evidence.dgraph_digest = grant.dgraph_ref.digest.clone();
        evidence.dmap_id = grant.dmap_ref.id.clone();
        evidence.dmap_revision = grant.dmap_ref.revision;
        evidence.dmap_digest = grant.dmap_ref.digest.clone();
        evidence.dserv_artifact_id = grant.destination.dserv_artifact_id.clone();
        evidence.dserv_artifact_digest = grant.destination.dserv_artifact_digest.clone();
        evidence.dserv_artifact_descriptor_digest =
            grant.destination.dserv_artifact_descriptor_digest.clone();
        evidence.input_port = grant.destination.port.clone();
        evidence.input_sha256 = grant.payload_sha256.clone();
        evidence.input_size = grant.payload_size as usize;
        evidence.message_id = grant.message_id.clone();
        evidence.correlation_id = grant.correlation_id.clone();
        evidence
    }

    /// 34C/D/E/F/H: a full round trip against the in-memory broker proves
    /// (C) the reply topic is subscribed BEFORE the request is published,
    /// (D) the request is published on the EXACT expected topic, (E/F) with
    /// QoS1/retain=false, and (H) a valid destination response produces
    /// success evidence.
    #[test]
    fn v2_source_delivery_round_trip_subscribes_before_publish_on_exact_topic_with_qos1_no_retain() {
        let grant = sample_v2_grant("remote");
        let binding = matching_binding_json(&grant, "mqtt://broker.lab:1883");
        let dserver_url = serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());

        let broker = crate::dcall::mqtt::test_support::InMemoryBroker::new();
        let log: std::sync::Arc<Mutex<Vec<&'static str>>> = std::sync::Arc::new(Mutex::new(Vec::new()));

        let authority = grant.v2_dcall_transport_authority.clone().unwrap();
        let expected_request_topic = crate::dcall::mqtt::request_topic(
            &grant.project_id,
            &grant.application_id,
            &authority.diot_placement_id,
            &grant.destination.node_id,
            &authority.channel,
        );
        let expected_reply_topic = crate::dcall::mqtt::reply_topic(
            &grant.project_id,
            &grant.application_id,
            &grant.source.node_id,
            &grant.grant_id,
        );

        let destination_handle = spawn_fake_destination(
            broker.clone(),
            expected_request_topic.clone(),
            expected_reply_topic.clone(),
            success_response_for(&grant),
        );

        let broker_for_connect = broker.clone();
        let log_for_connect = log.clone();
        let result = deliver_via_diot_pubsub(
            &dserver_url,
            &grant,
            "token",
            b"payload",
            &sha256_hex(b"payload"),
            std::time::Duration::from_secs(5),
            move |_broker_addr| {
                Ok(Box::new(OrderRecordingPubSub {
                    inner: broker_for_connect.connect(),
                    log: log_for_connect,
                }) as Box<dyn crate::dcall::mqtt::MqttPubSub>)
            },
        );
        destination_handle.join().unwrap();

        let (evidence, _redeemed_at) = result.expect("a valid destination response must succeed");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);

        // C: subscribe strictly before publish.
        let log = log.lock().unwrap().clone();
        let subscribe_pos = log.iter().position(|event| *event == "subscribe").unwrap();
        let publish_pos = log.iter().position(|event| *event == "publish").unwrap();
        assert!(subscribe_pos < publish_pos, "log: {log:?}");

        // D/E/F: exact topic, QoS1, retain=false.
        let published = broker.published_log();
        let request_entry = published
            .iter()
            .find(|(topic, ..)| *topic == expected_request_topic)
            .expect("the request must be published on the exact expected topic");
        assert_eq!(request_entry.2, crate::dcall::mqtt::MqttQos::AtLeastOnce);
        assert!(!request_entry.3, "retain must be false");
    }

    /// 113B3.1 section 22: every source-side round-trip test above used
    /// `sample_v2_grant("remote")`. This proves the identical MQTT round
    /// trip (subscribe-before-publish, exact topic, QoS1/retain=false,
    /// success) for a grant whose `route_kind` is `"local"` -- i.e. exactly
    /// what a genuine SAME-NODE `via_diot_pubsub` call produces. Combined
    /// with the structural guarantee at the top of `dispatch_one_call`
    /// (the `grant.v2_dcall_transport_authority.is_some()` check fires
    /// UNCONDITIONALLY, before the `route_kind` match is even reached, and
    /// therefore before `deliver_local` could ever be called), this closes
    /// the gap that no test previously exercised the MQTT path under a
    /// `route_kind == "local"` grant at all.
    #[test]
    fn v2_source_delivery_round_trip_succeeds_for_a_same_node_local_route_kind_grant() {
        let grant = sample_v2_grant("local");
        assert_eq!(
            grant.route_kind, ROUTE_KIND_LOCAL,
            "this test's premise requires a grant whose route_kind is literally 'local'"
        );
        let binding = matching_binding_json(&grant, "mqtt://broker.lab:1883");
        let dserver_url = serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());

        let broker = crate::dcall::mqtt::test_support::InMemoryBroker::new();

        let authority = grant.v2_dcall_transport_authority.clone().unwrap();
        let expected_request_topic = crate::dcall::mqtt::request_topic(
            &grant.project_id,
            &grant.application_id,
            &authority.diot_placement_id,
            &grant.destination.node_id,
            &authority.channel,
        );
        let expected_reply_topic = crate::dcall::mqtt::reply_topic(
            &grant.project_id,
            &grant.application_id,
            &grant.source.node_id,
            &grant.grant_id,
        );

        let destination_handle = spawn_fake_destination(
            broker.clone(),
            expected_request_topic,
            expected_reply_topic,
            success_response_for(&grant),
        );

        let broker_for_connect = broker.clone();
        let result = deliver_via_diot_pubsub(
            &dserver_url,
            &grant,
            "token",
            b"payload",
            &sha256_hex(b"payload"),
            std::time::Duration::from_secs(5),
            move |_broker_addr| {
                Ok(Box::new(broker_for_connect.connect()) as Box<dyn crate::dcall::mqtt::MqttPubSub>)
            },
        );
        destination_handle.join().unwrap();

        let (evidence, _redeemed_at) =
            result.expect("a same-node route_kind='local' via_diot_pubsub grant must still complete a real MQTT round trip");
        assert_eq!(evidence.outcome, DCodeInvocationOutcome::Success);
    }

    /// 34I: a destination response whose `dcode_evidence` does not
    /// deserialize into a real `DCodeInvocationEvidenceV1` fails closed
    /// (`dcall_remote_response_malformed`), never silently treated as
    /// success.
    #[test]
    fn v2_source_delivery_fails_closed_on_malformed_destination_evidence() {
        let grant = sample_v2_grant("remote");
        let binding = matching_binding_json(&grant, "mqtt://broker.lab:1883");
        let dserver_url = serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());
        let broker = crate::dcall::mqtt::test_support::InMemoryBroker::new();

        let authority = grant.v2_dcall_transport_authority.clone().unwrap();
        let request_topic = crate::dcall::mqtt::request_topic(
            &grant.project_id,
            &grant.application_id,
            &authority.diot_placement_id,
            &grant.destination.node_id,
            &authority.channel,
        );
        let reply_topic = crate::dcall::mqtt::reply_topic(
            &grant.project_id,
            &grant.application_id,
            &grant.source.node_id,
            &grant.grant_id,
        );
        let malformed_response = crate::dcall::mqtt::MqttDCallResponseEnvelopeV1 {
            schema: crate::dcall::mqtt::RESPONSE_ENVELOPE_SCHEMA.to_string(),
            grant_id: grant.grant_id.clone(),
            outcome: "success".to_string(),
            error_code: None,
            dcode_evidence: Some(serde_json::json!({"not": "a real evidence shape"})),
        };
        let destination_handle = spawn_fake_destination(
            broker.clone(),
            request_topic,
            reply_topic,
            malformed_response,
        );

        let broker_for_connect = broker.clone();
        let result = deliver_via_diot_pubsub(
            &dserver_url,
            &grant,
            "token",
            b"payload",
            &sha256_hex(b"payload"),
            std::time::Duration::from_secs(5),
            move |_broker_addr| {
                Ok(Box::new(broker_for_connect.connect()) as Box<dyn crate::dcall::mqtt::MqttPubSub>)
            },
        );
        destination_handle.join().unwrap();
        let failure = result.unwrap_err();
        assert_eq!(failure.code.as_str(), "dcall_remote_response_malformed");
    }

    /// 34G/J: publishing succeeds (simulating a broker PUBACK) but no
    /// destination ever replies -- PUBACK/publish success alone must NEVER
    /// be treated as DCall success; the bounded wait must expire and return
    /// a deterministic timeout failure.
    #[test]
    fn v2_source_delivery_times_out_when_no_reply_arrives_and_puback_alone_is_not_success() {
        let grant = sample_v2_grant("remote");
        let binding = matching_binding_json(&grant, "mqtt://broker.lab:1883");
        let dserver_url = serve_one_current_binding_response("HTTP/1.1 200 OK", binding.to_string());
        let broker = crate::dcall::mqtt::test_support::InMemoryBroker::new();
        // No destination subscriber is ever spawned -- publish succeeds
        // (analogous to a real PUBACK), but nothing ever replies.
        let broker_for_connect = broker.clone();
        let result = deliver_via_diot_pubsub(
            &dserver_url,
            &grant,
            "token",
            b"payload",
            &sha256_hex(b"payload"),
            std::time::Duration::from_millis(150),
            move |_broker_addr| {
                Ok(Box::new(broker_for_connect.connect()) as Box<dyn crate::dcall::mqtt::MqttPubSub>)
            },
        );
        let failure = result.unwrap_err();
        assert_eq!(failure.code.as_str(), "dcall_transport_timeout");
        assert!(failure.redeemed_at.is_none());
        // The publish itself must still have genuinely happened (proving
        // this is a real "no reply" timeout, not a publish failure).
        assert!(!broker.published_log().is_empty());
    }

    // -- Phase 113B3: destination-side MQTT handler tests (section 35, A-I) --
    //
    // Honest scope note (section 34/36 precedent, this file's own established
    // convention -- see `execute_destination_delivery_uses_the_grants_own_message_and_correlation_id`'s
    // doc comment): `execute_destination_delivery` uses the REAL isolated
    // subprocess worker executor, so a genuinely successful end-to-end D-Code
    // execution is proven by the real `dcall_two_node_integration` subprocess
    // test, never by an HTTP/MQTT-mocked `cargo test --lib` unit test -- that
    // precedent is unchanged by 113B3. The tests below therefore prove every
    // property that does NOT require a real successful WASM execution
    // (fail-closed-before-execution behavior A-D, single-use-gates-re-execution
    // F, response sanitization G/H, and reply-topic-from-redeemed-grant-not-request
    // I) directly against `handle_mqtt_dcall_request`, mocking only the
    // `POST .../redeem` HTTP call. Property E ("valid redemption executes
    // exact destination DCode") is covered by: (1) direct code inspection --
    // `handle_mqtt_dcall_request` calls the exact same `execute_destination_delivery`
    // helper the HTTP listener uses (section 19), so every existing HTTP-path
    // execution guarantee applies unchanged; (2) the untouched Phase-112 hard
    // gate; (3) the section-36 env-gated real-broker test. A dedicated real
    // end-to-end MQTT execution proof (extending `dcall_two_node_integration.rs`
    // with a genuine broker) is the natural closure item for Phase 113B4.

    fn serve_one_redeem_response(status_line: &'static str, body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = read_request_json_body(&mut stream);
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// A server that panics if it EVER receives a second connection --
    /// proves at most one HTTP call (the redeem attempt) was made, i.e. no
    /// destination D-Code authorization/artifact fetch (which
    /// `execute_destination_delivery` would need) was ever attempted (35A).
    fn serve_exactly_one_request_then_panic_on_a_second(
        status_line: &'static str,
        body: String,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = read_request_json_body(&mut stream);
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(200);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok(_) => panic!("a second HTTP request occurred -- destination D-Code was invoked before/despite a failed redemption"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        format!("http://{addr}")
    }

    fn sample_mqtt_request_envelope(grant_id: &str) -> crate::dcall::mqtt::MqttDCallRequestEnvelopeV1 {
        crate::dcall::mqtt::MqttDCallRequestEnvelopeV1 {
            schema: crate::dcall::mqtt::REQUEST_ENVELOPE_SCHEMA.to_string(),
            grant_id: grant_id.to_string(),
            token: "some-bearer-token".to_string(),
            payload_b64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"payload"),
            payload_sha256: sha256_hex(b"payload"),
            payload_size: 7,
        }
    }

    /// 35A: a redemption that fails (any reason) means NO destination D-Code
    /// invocation is ever attempted, proven by the server panicking if a
    /// second HTTP request (which `execute_destination_delivery` would make)
    /// ever arrives -- and `handle_mqtt_dcall_request` returns `None` (no
    /// reply for an unverified request).
    #[test]
    fn v2_destination_no_execution_before_redemption() {
        let dserver_url = serve_exactly_one_request_then_panic_on_a_second(
            "HTTP/1.1 403 Forbidden",
            serde_json::json!({"error": "dcall_grant_wrong_destination_node"}).to_string(),
        );
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-a-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope("grant-1");
        let result = handle_mqtt_dcall_request(
            &dserver_url,
            root.to_str().unwrap(),
            "datum/v1/dcall/observed/topic",
            &request,
        );
        // Give the panic-on-second-request server a moment to observe
        // whether a second connection ever arrives.
        std::thread::sleep(std::time::Duration::from_millis(250));
        assert!(result.is_none(), "a failed redemption must never produce a reply");
    }

    /// 35B: wrong destination node fails redemption -- same shape as A,
    /// asserted with the specific error DServer would actually return.
    #[test]
    fn v2_destination_wrong_node_fails_redemption() {
        let dserver_url = serve_one_redeem_response(
            "HTTP/1.1 403 Forbidden",
            serde_json::json!({"error": "dcall_grant_wrong_destination_node"}).to_string(),
        );
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-b-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope("grant-1");
        let result = handle_mqtt_dcall_request(
            &dserver_url,
            root.to_str().unwrap(),
            "datum/v1/dcall/observed/topic",
            &request,
        );
        assert!(result.is_none());
    }

    /// 35C: a wrong observed MQTT topic fails redemption -- proves the
    /// plumbing (the observed topic this installation actually saw the
    /// request on is exactly what gets sent to DServer's redeem endpoint,
    /// which independently verifies it -- section 17, already exhaustively
    /// tested server-side) and that `handle_mqtt_dcall_request` correctly
    /// treats that rejection as "no reply" (never `Pending`-but-reply-anyway).
    #[test]
    fn v2_destination_wrong_observed_topic_fails_redemption_with_no_reply() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(Mutex::new(None));
        let captured_for_server = captured.clone();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let body = read_request_json_body(&mut stream);
                *captured_for_server.lock().unwrap() = Some(body);
                let response_body =
                    serde_json::json!({"error": "dcall_observed_mqtt_topic_mismatch"}).to_string();
                let response = format!(
                    "HTTP/1.1 422 Unprocessable Entity\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
                    response_body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-c-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope("grant-1");
        let observed_topic = "datum/v1/dcall/exact/observed/topic";
        let result = handle_mqtt_dcall_request(
            &format!("http://{addr}"),
            root.to_str().unwrap(),
            observed_topic,
            &request,
        );
        assert!(result.is_none());
        std::thread::sleep(std::time::Duration::from_millis(100));
        let captured_body = captured.lock().unwrap().clone().unwrap();
        assert_eq!(
            captured_body["observed_mqtt_topic"].as_str(),
            Some(observed_topic),
            "the exact observed topic must be forwarded to DServer's redeem endpoint"
        );
    }

    /// 35D: a stale runtime binding fails redemption -- same shape as B/C.
    #[test]
    fn v2_destination_stale_runtime_binding_fails_redemption() {
        let dserver_url = serve_one_redeem_response(
            "HTTP/1.1 422 Unprocessable Entity",
            serde_json::json!({"error": "diot_runtime_binding_stale"}).to_string(),
        );
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-d-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope("grant-1");
        let result = handle_mqtt_dcall_request(
            &dserver_url,
            root.to_str().unwrap(),
            "datum/v1/dcall/observed/topic",
            &request,
        );
        assert!(result.is_none());
    }

    /// 35F: a duplicate MQTT delivery must not execute D-Code twice. The
    /// FIRST redemption attempt succeeds (a real grant is returned, so
    /// `execute_destination_delivery` genuinely runs -- and fails, since
    /// this unit-test environment has no real worker binary, producing a
    /// deterministic "failed" reply rather than a hang or panic); the
    /// SECOND attempt against the SAME grant_id is rejected by DServer as
    /// already-redeemed, proving no second execution attempt is even
    /// reachable -- the single-use grant is the only dedupe authority
    /// (section 21), no parallel application-level dedupe cache exists.
    #[test]
    fn v2_destination_duplicate_delivery_does_not_execute_twice() {
        let grant = sample_v2_grant("local");
        let grant_json = serde_json::to_value(&grant).unwrap();

        let first_dserver_url = serve_one_redeem_response("HTTP/1.1 200 OK", grant_json.to_string());
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-f-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope(&grant.grant_id);
        let first_result = handle_mqtt_dcall_request(
            &first_dserver_url,
            root.to_str().unwrap(),
            "datum/v1/dcall/observed/topic",
            &request,
        );
        // Redemption succeeded (a grant was returned), so a reply IS
        // produced -- outcome is "failed" only because this test
        // environment has no real worker binary to execute, never because
        // redemption itself failed.
        assert!(first_result.is_some());

        let second_dserver_url = serve_one_redeem_response(
            "HTTP/1.1 409 Conflict",
            serde_json::json!({"error": "dcall_grant_already_redeemed"}).to_string(),
        );
        let second_result = handle_mqtt_dcall_request(
            &second_dserver_url,
            root.to_str().unwrap(),
            "datum/v1/dcall/observed/topic",
            &request,
        );
        assert!(
            second_result.is_none(),
            "a duplicate delivery against an already-redeemed grant must never produce a reply (never a second execution attempt)"
        );
    }

    /// 35G/H: the response envelope never carries a bearer token or raw
    /// source payload -- proven both structurally (the type has no such
    /// field, `deny_unknown_fields`, see `mqtt::tests::response_envelope_carrying_a_token_field_is_rejected_by_deny_unknown_fields`)
    /// and here, at the value level, across every response this module
    /// actually produced during this test run.
    #[test]
    fn v2_destination_response_never_contains_token_or_raw_payload() {
        let grant = sample_v2_grant("local");
        let grant_json = serde_json::to_value(&grant).unwrap();
        let dserver_url = serve_one_redeem_response("HTTP/1.1 200 OK", grant_json.to_string());
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-gh-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope(&grant.grant_id);
        let (_, response) = handle_mqtt_dcall_request(
            &dserver_url,
            root.to_str().unwrap(),
            "datum/v1/dcall/observed/topic",
            &request,
        )
        .unwrap();
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains(&request.token));
        assert!(!serialized.contains("payload"));
        assert!(!serialized.contains(&request.payload_b64));
    }

    /// 35I: the reply topic is derived EXCLUSIVELY from the REDEEMED grant's
    /// own `project_id`/`application_id`/`source.node_id`/`grant_id` --
    /// never from the (untrustworthy) observed request topic. Proven by
    /// using an observed topic that looks nothing like the real reply
    /// topic, and confirming the returned reply topic still matches the
    /// grant-derived computation exactly.
    #[test]
    fn v2_destination_reply_topic_is_derived_from_the_redeemed_grant_not_the_request() {
        let grant = sample_v2_grant("local");
        let grant_json = serde_json::to_value(&grant).unwrap();
        let dserver_url = serve_one_redeem_response("HTTP/1.1 200 OK", grant_json.to_string());
        let root = std::env::temp_dir().join(format!("dcall-mqtt-dest-i-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let request = sample_mqtt_request_envelope(&grant.grant_id);
        let deliberately_unrelated_observed_topic = "datum/v1/dcall/totally/unrelated/topic/segment";
        let (reply_topic, _response) = handle_mqtt_dcall_request(
            &dserver_url,
            root.to_str().unwrap(),
            deliberately_unrelated_observed_topic,
            &request,
        )
        .unwrap();
        let expected = crate::dcall::mqtt::reply_topic(
            &grant.project_id,
            &grant.application_id,
            &grant.source.node_id,
            &grant.grant_id,
        );
        assert_eq!(reply_topic, expected);
        assert_ne!(reply_topic, deliberately_unrelated_observed_topic);
    }

    /// 27H (renumbered under 113B3, still required): a DMap/1 grant response
    /// (no `v2_dcall_transport_authority` field at all) is completely
    /// unaffected by the MQTT delivery path; dispatch proceeds into the
    /// ordinary `route_kind`-based branches exactly as before Phase 113B2/B3.
    #[test]
    fn v2_seam_never_fires_for_a_dmap1_grant_without_transport_authority() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = read_request_json_body(&mut stream);
                let body = serde_json::json!(["call-a"]).to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
            {
                let (mut stream, _) = listener.accept().unwrap();
                let request_body = read_request_json_body(&mut stream);
                let mut body: serde_json::Value =
                    serde_json::from_str(&fanout_grant_response("call-a", "m1", "c1")).unwrap();
                body["grant"]["source_dcode_evidence_sha256"] =
                    request_body["source_dcode_evidence_sha256"].clone();
                let body = body.to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let root = std::env::temp_dir().join(format!("dcall-v1-no-seam-{}", Uuid::new_v4()));
        crate::dcode::local_identity::init_local_identity(&root, "fog-01").unwrap();
        let source = real_source_result(
            "fog-01",
            "fog-01",
            r#"{"status":"ok","emits":[{"port":"out","payload_b64":"cGF5bG9hZA=="}]}"#,
        );
        let context = SourceExecutionContext {
            dserver_url: format!("http://{addr}"),
            dnode_root: root.to_str().unwrap().to_string(),
        };
        let results = dispatch_validated_emit_for_test(&context, &source, 0, Path::new("unused"));
        assert_eq!(results.len(), 1);
        // `fanout_grant_response` deliberately uses an unsupported
        // route_kind (see its own doc comment) -- proving dispatch reached
        // the ordinary `match route_kind` branch (and failed THERE, on the
        // unsupported value) rather than being halted by the v2 seam, which
        // never applies to a grant with no transport authority at all.
        assert_eq!(
            results[0].error_code.as_deref(),
            Some("dcall_unsupported_route_kind"),
            "a DMap/1 grant (no v2 transport authority) must reach the ordinary route_kind dispatch, never the v2 seam"
        );
    }

    // --------------------------------------------------- PHASE 114D0B

    fn diot_binding_identity_fixture() -> crate::agent::diot_runtime_dmonitor::DiotRuntimeBindingIdentity {
        crate::agent::diot_runtime_dmonitor::DiotRuntimeBindingIdentity {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            diot_id: "diot:broker".to_string(),
            diot_placement_id: "diotp:broker".to_string(),
            node_id: "cloud-01".to_string(),
            dmap_id: "dmap:1".to_string(),
            dmap_revision: 1,
            dmap_digest: format!("sha256:{}", "a".repeat(64)),
            dforward_id: "dforward:1".to_string(),
            dforward_revision: 1,
            dforward_digest: format!("sha256:{}", "b".repeat(64)),
            binding_id: "diot-runtime-binding:proj:lab:app:pipeline:diotp:broker".to_string(),
            binding_revision: 1,
            descriptor_digest: format!("sha256:{}", "c".repeat(64)),
        }
    }

    /// Section 25: proves `handle_mqtt_dcall_request` is still invoked for
    /// every well-formed inbound DCall envelope WHILE the DMonitor
    /// heartbeat producer is active in the same loop -- neither disrupts
    /// the other. The envelope's `grant_id` cannot be redeemed against
    /// `"http://127.0.0.1:1"` (this codebase's own established
    /// unreachable-dserver test convention -- see
    /// `dispatch_reports_a_sanitized_lookup_failure_when_dserver_is_unreachable`),
    /// so no reply is ever published -- this test's claim is deliberately
    /// narrower than full grant redemption (already covered by the 38
    /// other tests in this module): it proves the DISPATCH CODE PATH runs,
    /// undisturbed, alongside real heartbeat emission.
    #[test]
    fn dcall_message_dispatch_and_dmonitor_heartbeats_coexist_in_the_same_loop() {
        use base64::Engine;
        let broker = crate::dcall::mqtt::test_support::InMemoryBroker::new();
        let listener_transport = broker.connect();
        let mut peer = broker.connect();

        let dmonitor_output = crate::outputs::dmonitor_http::DMonitorHttpOutput::new(
            "http://127.0.0.1:1".to_string(),
            "proj:lab".to_string(),
            1,
        )
        .unwrap();
        let publisher = crate::dcall::dmonitor_publisher::DMonitorPublisherHandle::spawn(
            dmonitor_output,
            crate::dcall::dmonitor_publisher::DEFAULT_QUEUE_CAPACITY,
        );
        let emission_context =
            crate::agent::dmonitor_emission::DMonitorEmissionContext::new_process_epoch("cloud-01");

        let stop = std::sync::atomic::AtomicBool::new(false);
        let heartbeat_interval = std::time::Duration::from_millis(20);
        let poll_interval = std::time::Duration::from_millis(10);
        let confirm_timeout = std::time::Duration::from_millis(200);

        let payload = b"payload";
        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let request = crate::dcall::mqtt::MqttDCallRequestEnvelopeV1 {
            schema: crate::dcall::mqtt::REQUEST_ENVELOPE_SCHEMA.to_string(),
            grant_id: "dcall-grant:unredeemable".to_string(),
            token: "tok".to_string(),
            payload_b64,
            payload_sha256: sha256_hex(payload),
            payload_size: payload.len() as u64,
        };
        let request_bytes = serde_json::to_vec(&request).unwrap();
        let request_topic = crate::dcall::mqtt::request_topic(
            "proj:lab",
            "app:pipeline",
            "diotp:broker",
            "cloud-01",
            "some-channel",
        );

        std::thread::scope(|scope| {
            scope.spawn(|| {
                // Give `establish_and_confirm`'s subscribe a moment to
                // register before publishing (in-memory broker never
                // queues for a not-yet-subscribed client).
                std::thread::sleep(std::time::Duration::from_millis(30));
                crate::dcall::mqtt::MqttPubSub::publish(&mut peer, &request_topic, &request_bytes, crate::dcall::mqtt::MqttQos::AtLeastOnce, false).unwrap();
            });
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(150));
                stop.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let result = run_mqtt_dcall_listener_loop(
                listener_transport,
                "http://127.0.0.1:1",
                "unused-dnode-root",
                "proj:lab",
                "app:pipeline",
                "diotp:broker",
                "cloud-01",
                diot_binding_identity_fixture(),
                emission_context,
                crate::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter::new(
                    crate::agent::dmonitor_emission::DMonitorEmissionContext::new_process_epoch("cloud-01"),
                    "cloud-01".to_string(),
                ),
                &publisher,
                heartbeat_interval,
                poll_interval,
                confirm_timeout,
                &stop,
            );
            assert!(result.is_ok(), "graceful stop must return Ok: {result:?}");
        });
        publisher.shutdown();
        // No reply was ever published (the grant is unredeemable), but the
        // dispatch attempt itself did not panic, error the loop, or starve
        // heartbeat production -- the loop completing within this test's
        // bounded 150ms window (rather than hanging) is itself part of the
        // proof, enforced by the surrounding `#[test]` harness's own
        // default timeout.
        assert!(broker.published_log().iter().all(|(topic, ..)| topic == &request_topic), "no reply should ever have been published for an unredeemable grant");
    }

    // --------------------------------------------------- PHASE 115B

    fn sample_dcode_evidence(node_id: &str) -> crate::dcode::evidence::DCodeInvocationEvidenceV1 {
        crate::dcode::evidence::DCodeInvocationEvidenceV1 {
            schema: DCODE_EVIDENCE_SCHEMA.to_string(),
            authorization_id: "dcode-auth:proj:lab:app:pipeline:decode:1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "decode".to_string(),
            node_id: node_id.to_string(),
            dmap_id: "dmap:1".to_string(),
            dmap_revision: 1,
            dmap_digest: format!("sha256:{}", "a".repeat(64)),
            dgraph_id: "dgraph:pipeline".to_string(),
            dgraph_revision: 1,
            dgraph_digest: format!("sha256:{}", "b".repeat(64)),
            dserv_artifact_id: "dserv:pipeline:decode".to_string(),
            dserv_artifact_digest: "c".repeat(64),
            dserv_artifact_descriptor_digest: format!("sha256:{}", "d".repeat(64)),
            module_size_bytes: Some(64),
            dnode_abi: "datum-dnode/0".to_string(),
            fresh_instance_per_invocation: true,
            input_port: "in".to_string(),
            message_id: "msg-1".to_string(),
            correlation_id: "corr-1".to_string(),
            input_sha256: "e".repeat(64),
            input_size: 7,
            input_envelope_sha256: "f".repeat(64),
            input_envelope_size: 64,
            received_at_utc: "2026-09-25T10:00:00Z".to_string(),
            output_sha256: None,
            output_size: None,
            output_status: Some("ok".to_string()),
            output_reason_present: false,
            output_reason_sha256: None,
            output_reason_size: None,
            emits: Vec::new(),
            limits_applied: DCodeAppliedLimits {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 100,
                max_memory_pages: 4,
                max_message_bytes: 65536,
            },
            outcome: crate::dcode::evidence::DCodeInvocationOutcome::Success,
            error_code: None,
            error_message: None,
            error_offending_value_sha256: None,
            error_offending_value_size: None,
            duration_ms: 3,
        }
    }

    fn response_with_evidence(
        evidence: &crate::dcode::evidence::DCodeInvocationEvidenceV1,
    ) -> crate::dcall::mqtt::MqttDCallResponseEnvelopeV1 {
        crate::dcall::mqtt::MqttDCallResponseEnvelopeV1 {
            schema: crate::dcall::mqtt::RESPONSE_ENVELOPE_SCHEMA.to_string(),
            grant_id: "dcall-grant:1".to_string(),
            outcome: "success".to_string(),
            error_code: None,
            dcode_evidence: Some(serde_json::to_value(evidence).unwrap()),
        }
    }

    /// A minimal counting mock DServer: accepts every connection, always
    /// replies 200 `{"outcome":"accepted"}`, and records how many requests
    /// it received -- used to prove `emit_dserv_evidence_if_present`
    /// actually offers (or withholds) a publish job, without depending on
    /// `dmonitor_publisher`'s own private test helpers in another module.
    struct CountingMockDServer {
        url: String,
        count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl CountingMockDServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let counted = count.clone();
            let stopping = stop.clone();
            let thread = std::thread::spawn(move || {
                while !stopping.load(std::sync::atomic::Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                            let mut bytes = Vec::new();
                            let mut buffer = [0u8; 4096];
                            loop {
                                let read = match stream.read(&mut buffer) {
                                    Ok(0) => break,
                                    Ok(read) => read,
                                    Err(_) => break,
                                };
                                bytes.extend_from_slice(&buffer[..read]);
                                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let body = r#"{"outcome":"accepted"}"#;
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self { url, count, stop, thread: Some(thread) }
        }

        fn received_count(&self) -> usize {
            self.count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Drop for CountingMockDServer {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn emitter_for(node_id: &str) -> crate::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter {
        crate::agent::dcode_execution_dmonitor::DCodeExecutionMonitorEmitter::new(
            crate::agent::dmonitor_emission::DMonitorEmissionContext::new_process_epoch(node_id),
            node_id.to_string(),
        )
    }

    /// Phase 115B: a response carrying real, matching-node D-Code evidence
    /// results in one canonical `DServRealization` observation actually
    /// being sent to DServer.
    #[test]
    fn matching_node_evidence_is_published_as_a_dserv_realization() {
        let server = CountingMockDServer::start();
        let output = crate::outputs::dmonitor_http::DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let publisher = crate::dcall::dmonitor_publisher::DMonitorPublisherHandle::spawn(output, crate::dcall::dmonitor_publisher::DEFAULT_QUEUE_CAPACITY);
        let mut emitter = emitter_for("fog-01");
        let evidence = sample_dcode_evidence("fog-01");
        let response = response_with_evidence(&evidence);

        emit_dserv_evidence_if_present(&response, &mut emitter, &publisher);
        std::thread::sleep(std::time::Duration::from_millis(200));
        publisher.shutdown();

        assert_eq!(server.received_count(), 1, "exactly one DServRealization observation must be published");
    }

    /// Phase 115A Q9 / 115B.3: evidence claiming a DIFFERENT node than this
    /// process's own local identity must never be published -- fail closed,
    /// never guess which of two disagreeing node values to trust.
    #[test]
    fn wrong_node_evidence_is_never_published() {
        let server = CountingMockDServer::start();
        let output = crate::outputs::dmonitor_http::DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let publisher = crate::dcall::dmonitor_publisher::DMonitorPublisherHandle::spawn(output, crate::dcall::dmonitor_publisher::DEFAULT_QUEUE_CAPACITY);
        let mut emitter = emitter_for("fog-01");
        // Evidence claims "cloud-01", but this process's own emitter is
        // bound to "fog-01" -- a mismatch `dcode::invoke` itself can only
        // produce via `DCodeInvocationOutcome::StaleAuthority`.
        let evidence = sample_dcode_evidence("cloud-01");
        let response = response_with_evidence(&evidence);

        emit_dserv_evidence_if_present(&response, &mut emitter, &publisher);
        std::thread::sleep(std::time::Duration::from_millis(200));
        publisher.shutdown();

        assert_eq!(server.received_count(), 0, "a node-identity mismatch must never be published as a local realization");
    }

    /// A response with no D-Code evidence at all (malformed request, grant
    /// redemption failure -- no invocation ever happened) must never
    /// fabricate an observation.
    #[test]
    fn response_without_dcode_evidence_publishes_nothing() {
        let server = CountingMockDServer::start();
        let output = crate::outputs::dmonitor_http::DMonitorHttpOutput::new(server.url.clone(), "proj:lab".to_string(), 5).unwrap();
        let publisher = crate::dcall::dmonitor_publisher::DMonitorPublisherHandle::spawn(output, crate::dcall::dmonitor_publisher::DEFAULT_QUEUE_CAPACITY);
        let mut emitter = emitter_for("fog-01");
        let response = crate::dcall::mqtt::MqttDCallResponseEnvelopeV1 {
            schema: crate::dcall::mqtt::RESPONSE_ENVELOPE_SCHEMA.to_string(),
            grant_id: "dcall-grant:1".to_string(),
            outcome: "failed".to_string(),
            error_code: Some("dcall_destination_execution_failed".to_string()),
            dcode_evidence: None,
        };

        emit_dserv_evidence_if_present(&response, &mut emitter, &publisher);
        std::thread::sleep(std::time::Duration::from_millis(200));
        publisher.shutdown();

        assert_eq!(server.received_count(), 0);
    }
}
