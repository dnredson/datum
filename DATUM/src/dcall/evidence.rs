//! D-Call delivery evidence (ADR-0022, sections 19-20). Follows the exact
//! sanitization discipline `dcode::evidence` established for Phase 111:
//! digests/sizes/deterministic error codes only, never a full arbitrary
//! payload and never a guest- or remote-controlled string verbatim.
//!
//! This type is deliberately built ENTIRELY from the redeemed
//! `DCallDeliveryGrantV1` (which itself carries only canonical digests/ids,
//! never a raw endpoint URL) plus a digest reference to the destination's
//! own `DCodeInvocationEvidenceV1` -- never a full copy of it, per section
//! 20 ("do not duplicate the complete D-Code evidence inside D-Call
//! evidence if a digest/ref is enough").
//!
//! A remote delivery failure necessarily involves a raw HTTP response body
//! that DATUM does not control (it could contain absolutely anything --
//! including, in the worst case, guest-influenced content laundered through
//! a compromised or buggy destination listener). `error_diagnostic_sha256`
//! records a digest of that raw diagnostic text so a human with access to
//! the original transcript could still correlate it, but the text itself
//! never appears in evidence -- see `dcall-payload-leakage-audit.txt`.

use serde::{Deserialize, Serialize};

use crate::dcall::grant::{CanonicalContractReferenceV1, DCallRouteEndpointV1};
use crate::dcode::identity::sha256_hex;

pub const DCALL_DELIVERY_EVIDENCE_SCHEMA: &str = "datum.dcall-delivery-evidence/1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DCallDeliveryOutcome {
    Success,
    Failed,
}

/// Deterministic, non-content-bearing failure classification. Never embeds
/// a raw remote response body, guest-controlled string, or arbitrary error
/// text -- see the module doc for why `error_diagnostic_sha256` exists
/// instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DCallDeliveryErrorCode {
    DCallSourceEvidenceMismatch,
    DCallLookupFailed,
    DCallGrantRequestFailed,
    DCallSourceExecutionStale,
    DCallPayloadExceedsSourceMessageLimit,
    DCallPayloadExceedsDestinationMessageLimit,
    DCallGrantRedeemFailed,
    DCallUnsupportedRouteKind,
    DCallEndpointFetchFailed,
    DCallEndpointAuthorityMismatch,
    DCallTransportTimeout,
    DCallTransportError,
    DCallRemoteResponseMalformed,
    DCallDestinationExecutionFailed,
    /// I1B: the source invocation this dispatch was asked to act on did not
    /// itself succeed.
    DCallSourceInvocationNotSuccessful,
    /// I1B: the claimed emit port does not appear in the source invocation's
    /// OWN recorded evidence emit list.
    DCallSourceEmitNotFound,
    /// I1B/AN: the actual bytes being dispatched do not match the digest/size
    /// the source invocation's evidence recorded for that emit -- proves
    /// arbitrary host bytes cannot masquerade as a validated source emit.
    DCallSourceEmitPayloadMismatch,
    /// I1C: this installation's own local D-Node identity could not be
    /// loaded (has `smartsentinel-dcode-init` been run for this dnode_root?).
    DCallSourceLocalIdentityLoadFailed,
    /// I1C: this process's own local D-Node identity does not match the node
    /// the source invocation's evidence claims to have run on -- closes the
    /// source-node-spoofing gap.
    DCallSourceLocalIdentityMismatch,
    /// I3: the destination's own `DCodeInvocationEvidenceV1` (whether
    /// obtained locally or deserialized from a remote listener's response)
    /// does not match what the just-redeemed grant actually authorized.
    DCallDestinationEvidenceMismatch,
    /// I6: a remote listener's response exceeded the bounded read cap.
    DCallRemoteResponseTooLarge,
    /// Phase 113B2 (section 19, mandatory fail-closed seam): the grant
    /// carries typed DMap/2 `via_diot_pubsub` transport authority --
    /// realization authorization succeeded, but no MQTT runtime transport
    /// was implemented until Phase 113B3 wired real delivery. Retained
    /// (never removed) for reload compatibility with any evidence persisted
    /// during 113B2 and as a defensive code for a realization_kind DServer
    /// might accept in the future that still has no implemented transport --
    /// it is no longer reachable for `via_diot_pubsub` specifically, since
    /// that path now has real MQTT delivery.
    DCallDiotTransportNotImplemented,
    /// Phase 113B3: `deliver_via_diot_pubsub` was invoked for a grant with no
    /// `v2_dcall_transport_authority` -- an internal-consistency guard, not
    /// expected to be reachable in production (the dispatch seam only calls
    /// this function when the authority is `Some`).
    DCallDiotTransportAuthorityMissing,
    /// Phase 113B3 (section 8): fetching the CURRENT DIoT runtime binding
    /// failed (network error, or DServer reports it missing/stale).
    DCallDiotRuntimeBindingFetchFailed,
    /// Phase 113B3 (section 8): the CURRENT runtime binding's identity
    /// (binding_id/binding_revision/descriptor_digest/transport/placement)
    /// does not match the grant's bound transport authority -- closes the
    /// "grant issued under binding N, binding becomes N+1" race BEFORE any
    /// MQTT connection is opened.
    DCallDiotRuntimeBindingAuthorityMismatch,
    /// Phase 113B3 (section 4): the current binding's `broker_uri` declares
    /// a scheme (e.g. `mqtts://`) not yet supported for actual delivery in
    /// this release -- deterministic, distinct from a generic transport
    /// error.
    DCallDiotBrokerSchemeUnsupported,
}

impl DCallDeliveryErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DCallSourceEvidenceMismatch => "dcall_source_evidence_mismatch",
            Self::DCallLookupFailed => "dcall_lookup_failed",
            Self::DCallGrantRequestFailed => "dcall_grant_request_failed",
            Self::DCallSourceExecutionStale => "dcall_source_execution_stale",
            Self::DCallPayloadExceedsSourceMessageLimit => {
                "dcall_payload_exceeds_source_message_limit"
            }
            Self::DCallPayloadExceedsDestinationMessageLimit => {
                "dcall_payload_exceeds_destination_message_limit"
            }
            Self::DCallGrantRedeemFailed => "dcall_grant_redeem_failed",
            Self::DCallUnsupportedRouteKind => "dcall_unsupported_route_kind",
            Self::DCallEndpointFetchFailed => "dcall_endpoint_fetch_failed",
            Self::DCallEndpointAuthorityMismatch => "dcall_endpoint_authority_mismatch",
            Self::DCallTransportTimeout => "dcall_transport_timeout",
            Self::DCallTransportError => "dcall_transport_error",
            Self::DCallRemoteResponseMalformed => "dcall_remote_response_malformed",
            Self::DCallDestinationExecutionFailed => "dcall_destination_execution_failed",
            Self::DCallSourceInvocationNotSuccessful => "dcall_source_invocation_not_successful",
            Self::DCallSourceEmitNotFound => "dcall_source_emit_not_found",
            Self::DCallSourceEmitPayloadMismatch => "dcall_source_emit_payload_mismatch",
            Self::DCallSourceLocalIdentityLoadFailed => "dcall_source_local_identity_load_failed",
            Self::DCallSourceLocalIdentityMismatch => "dcall_source_local_identity_mismatch",
            Self::DCallDestinationEvidenceMismatch => "dcall_destination_evidence_mismatch",
            Self::DCallRemoteResponseTooLarge => "dcall_remote_response_too_large",
            Self::DCallDiotTransportNotImplemented => "dcall_diot_transport_not_implemented",
            Self::DCallDiotTransportAuthorityMissing => "dcall_diot_transport_authority_missing",
            Self::DCallDiotRuntimeBindingFetchFailed => "dcall_diot_runtime_binding_fetch_failed",
            Self::DCallDiotRuntimeBindingAuthorityMismatch => {
                "dcall_diot_runtime_binding_authority_mismatch"
            }
            Self::DCallDiotBrokerSchemeUnsupported => "dcall_diot_broker_scheme_unsupported",
        }
    }
}

/// Full delivery-attempt evidence record (section 19). Built exclusively
/// from the REDEEMED grant's own bound fields -- never re-derived or
/// re-fetched afterward, so evidence always reflects exactly what authority
/// the delivery actually executed under.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DCallDeliveryEvidenceV1 {
    pub schema: String,
    pub delivery_id: String,
    pub grant_id: String,

    pub project_id: String,
    pub application_id: String,
    pub call_id: String,

    pub dgraph_ref: CanonicalContractReferenceV1,
    pub dmap_ref: CanonicalContractReferenceV1,

    pub source: DCallRouteEndpointV1,
    pub destination: DCallRouteEndpointV1,

    pub message_id: String,
    pub correlation_id: String,

    pub payload_sha256: String,
    pub payload_size: u64,

    /// Independent review fix v5 (section 9/12): opaque digest reference to
    /// the exact GOVERNED source `DCodeInvocationEvidenceV1` that produced
    /// this delivery's emit -- copied verbatim from the redeemed grant's own
    /// bound field (never independently recomputed here), so
    /// `source_dcode_evidence_sha256 == grant.source_dcode_evidence_sha256`
    /// by construction. `None` only for a pre-grant failure, where no grant
    /// (and therefore no bound source evidence reference) exists yet.
    #[serde(default)]
    pub source_dcode_evidence_sha256: Option<String>,

    pub route_kind: String,
    #[serde(default)]
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub endpoint_revision: Option<u64>,
    #[serde(default)]
    pub endpoint_descriptor_digest: Option<String>,

    pub issued_at: String,
    /// The LOCAL timestamp this dispatching process observed successful
    /// redemption at -- DServer's redemption response does not itself
    /// return a `redeemed_at` field (only the grant's original bound
    /// fields), so this is honestly the caller's own clock, not a
    /// server-attested value.
    #[serde(default)]
    pub redeemed_at: Option<String>,

    pub outcome: DCallDeliveryOutcome,
    #[serde(default)]
    pub error_code: Option<String>,
    /// Digest of a raw, potentially remote/guest-influenced diagnostic
    /// string -- NEVER the string itself. See module doc.
    #[serde(default)]
    pub error_diagnostic_sha256: Option<String>,

    /// Digest of the destination's own `DCodeInvocationEvidenceV1` JSON
    /// (never the full record duplicated here -- section 20).
    #[serde(default)]
    pub destination_dcode_evidence_sha256: Option<String>,

    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
pub fn build_success_evidence(
    delivery_id: String,
    grant: &crate::dcall::grant::DCallDeliveryGrantV1,
    redeemed_at: String,
    destination_dcode_evidence_json: &[u8],
    duration_ms: u64,
) -> DCallDeliveryEvidenceV1 {
    DCallDeliveryEvidenceV1 {
        schema: DCALL_DELIVERY_EVIDENCE_SCHEMA.to_string(),
        delivery_id,
        grant_id: grant.grant_id.clone(),
        project_id: grant.project_id.clone(),
        application_id: grant.application_id.clone(),
        call_id: grant.call_id.clone(),
        dgraph_ref: grant.dgraph_ref.clone(),
        dmap_ref: grant.dmap_ref.clone(),
        source: grant.source.clone(),
        destination: grant.destination.clone(),
        message_id: grant.message_id.clone(),
        correlation_id: grant.correlation_id.clone(),
        payload_sha256: grant.payload_sha256.clone(),
        payload_size: grant.payload_size,
        source_dcode_evidence_sha256: Some(grant.source_dcode_evidence_sha256.clone()),
        route_kind: grant.route_kind.clone(),
        endpoint_id: grant.endpoint_id.clone(),
        endpoint_revision: grant.endpoint_revision,
        endpoint_descriptor_digest: grant.endpoint_descriptor_digest.clone(),
        issued_at: grant.issued_at.clone(),
        redeemed_at: Some(redeemed_at),
        outcome: DCallDeliveryOutcome::Success,
        error_code: None,
        error_diagnostic_sha256: None,
        destination_dcode_evidence_sha256: Some(sha256_hex(destination_dcode_evidence_json)),
        duration_ms: Some(duration_ms),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_grant_bound_failure_evidence(
    delivery_id: String,
    grant: &crate::dcall::grant::DCallDeliveryGrantV1,
    redeemed_at: Option<String>,
    error_code: DCallDeliveryErrorCode,
    raw_diagnostic: Option<&str>,
    duration_ms: u64,
) -> DCallDeliveryEvidenceV1 {
    DCallDeliveryEvidenceV1 {
        schema: DCALL_DELIVERY_EVIDENCE_SCHEMA.to_string(),
        delivery_id,
        grant_id: grant.grant_id.clone(),
        project_id: grant.project_id.clone(),
        application_id: grant.application_id.clone(),
        call_id: grant.call_id.clone(),
        dgraph_ref: grant.dgraph_ref.clone(),
        dmap_ref: grant.dmap_ref.clone(),
        source: grant.source.clone(),
        destination: grant.destination.clone(),
        message_id: grant.message_id.clone(),
        correlation_id: grant.correlation_id.clone(),
        payload_sha256: grant.payload_sha256.clone(),
        payload_size: grant.payload_size,
        source_dcode_evidence_sha256: Some(grant.source_dcode_evidence_sha256.clone()),
        route_kind: grant.route_kind.clone(),
        endpoint_id: grant.endpoint_id.clone(),
        endpoint_revision: grant.endpoint_revision,
        endpoint_descriptor_digest: grant.endpoint_descriptor_digest.clone(),
        issued_at: grant.issued_at.clone(),
        redeemed_at,
        outcome: DCallDeliveryOutcome::Failed,
        error_code: Some(error_code.as_str().to_string()),
        error_diagnostic_sha256: raw_diagnostic.map(|text| sha256_hex(text.as_bytes())),
        destination_dcode_evidence_sha256: None,
        duration_ms: Some(duration_ms),
    }
}

/// Failure BEFORE a grant even existed (e.g. call lookup or grant-request
/// itself failed) -- there is no grant to bind fields from, so only the
/// caller-known selectors are recorded.
pub fn build_pre_grant_failure_evidence(
    delivery_id: String,
    project_id: &str,
    application_id: &str,
    call_id: &str,
    error_code: DCallDeliveryErrorCode,
    raw_diagnostic: Option<&str>,
    duration_ms: u64,
) -> DCallDeliveryEvidenceV1 {
    let empty_endpoint = DCallRouteEndpointV1 {
        service_id: String::new(),
        port: String::new(),
        node_id: String::new(),
        dserv_artifact_id: String::new(),
        dserv_artifact_digest: String::new(),
        dserv_artifact_descriptor_digest: String::new(),
    };
    let empty_ref = CanonicalContractReferenceV1 {
        id: String::new(),
        revision: 0,
        digest: String::new(),
    };
    DCallDeliveryEvidenceV1 {
        schema: DCALL_DELIVERY_EVIDENCE_SCHEMA.to_string(),
        delivery_id,
        grant_id: String::new(),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        call_id: call_id.to_string(),
        dgraph_ref: empty_ref.clone(),
        dmap_ref: empty_ref,
        source: empty_endpoint.clone(),
        destination: empty_endpoint,
        message_id: String::new(),
        correlation_id: String::new(),
        payload_sha256: String::new(),
        payload_size: 0,
        source_dcode_evidence_sha256: None,
        route_kind: String::new(),
        endpoint_id: None,
        endpoint_revision: None,
        endpoint_descriptor_digest: None,
        issued_at: String::new(),
        redeemed_at: None,
        outcome: DCallDeliveryOutcome::Failed,
        error_code: Some(error_code.as_str().to_string()),
        error_diagnostic_sha256: raw_diagnostic.map(|text| sha256_hex(text.as_bytes())),
        destination_dcode_evidence_sha256: None,
        duration_ms: Some(duration_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_grant() -> crate::dcall::grant::DCallDeliveryGrantV1 {
        let endpoint = DCallRouteEndpointV1 {
            service_id: "svc:dest".to_string(),
            port: "in".to_string(),
            node_id: "fog-01".to_string(),
            dserv_artifact_id: "artifact:dest".to_string(),
            dserv_artifact_digest: "sha256:aa".to_string(),
            dserv_artifact_descriptor_digest: "sha256:bb".to_string(),
        };
        crate::dcall::grant::DCallDeliveryGrantV1 {
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
            payload_sha256: "e".repeat(64),
            payload_size: 4,
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

    #[test]
    fn success_evidence_never_contains_the_destination_evidence_payload_itself() {
        let grant = sample_grant();
        let dcode_evidence_json =
            br#"{"outcome":"success","some_field":"not evidence content, just json bytes"}"#;
        let evidence = build_success_evidence(
            "delivery-1".to_string(),
            &grant,
            "2026-01-01T00:00:01Z".to_string(),
            dcode_evidence_json,
            12,
        );
        let serialized = serde_json::to_string(&evidence).unwrap();
        assert!(!serialized.contains("not evidence content"));
        assert_eq!(
            evidence.destination_dcode_evidence_sha256,
            Some(sha256_hex(dcode_evidence_json))
        );
        assert!(matches!(evidence.outcome, DCallDeliveryOutcome::Success));
    }

    #[test]
    fn failure_evidence_never_leaks_a_marker_from_a_raw_remote_diagnostic() {
        let grant = sample_grant();
        let marker = "SUPER_SECRET_DCALL_DIAGNOSTIC_MARKER_1234567890";
        let raw_diagnostic =
            format!("remote listener returned status=500 body={{\"reason\":\"{marker}\"}}");
        let evidence = build_grant_bound_failure_evidence(
            "delivery-2".to_string(),
            &grant,
            None,
            DCallDeliveryErrorCode::DCallTransportError,
            Some(&raw_diagnostic),
            5,
        );
        let serialized = serde_json::to_string(&evidence).unwrap();
        assert!(
            !serialized.contains(marker),
            "evidence JSON leaked the marker: {serialized}"
        );
        assert_eq!(
            evidence.error_code.as_deref(),
            Some("dcall_transport_error")
        );
        assert_eq!(
            evidence.error_diagnostic_sha256,
            Some(sha256_hex(raw_diagnostic.as_bytes()))
        );
    }

    #[test]
    fn pre_grant_failure_evidence_has_no_grant_fields_but_still_no_leak() {
        let marker = "SUPER_SECRET_DCALL_PRE_GRANT_MARKER";
        let raw = format!("lookup failed: {marker}");
        let evidence = build_pre_grant_failure_evidence(
            "delivery-3".to_string(),
            "proj:lab",
            "app:a",
            "",
            DCallDeliveryErrorCode::DCallLookupFailed,
            Some(&raw),
            1,
        );
        let serialized = serde_json::to_string(&evidence).unwrap();
        assert!(!serialized.contains(marker));
        assert_eq!(evidence.grant_id, "");
        assert_eq!(evidence.error_code.as_deref(), Some("dcall_lookup_failed"));
    }

    #[test]
    fn error_code_strings_are_stable() {
        assert_eq!(
            DCallDeliveryErrorCode::DCallTransportTimeout.as_str(),
            "dcall_transport_timeout"
        );
        assert_eq!(
            DCallDeliveryErrorCode::DCallGrantRedeemFailed.as_str(),
            "dcall_grant_redeem_failed"
        );
        assert_eq!(
            DCallDeliveryErrorCode::DCallDestinationExecutionFailed.as_str(),
            "dcall_destination_execution_failed"
        );
    }
}
