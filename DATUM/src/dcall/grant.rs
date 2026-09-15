//! Mirrors DServer's `core::dcall::DCallDeliveryGrantV1` wire shape exactly
//! (ADR-0022), following the same independent-mirroring discipline as
//! `dcode::authorization` (ADR-0007: no shared crate). `#[serde(deny_unknown_fields)]`
//! for the same reason `dcode::authorization` uses it: a future dserver-side
//! field added without a matching DATUM-side update must fail loudly, never
//! silently drop data a security check might depend on.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalContractReferenceV1 {
    pub id: String,
    pub revision: u64,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCallRouteEndpointV1 {
    pub service_id: String,
    pub port: String,
    pub node_id: String,
    pub dserv_artifact_id: String,
    pub dserv_artifact_digest: String,
    pub dserv_artifact_descriptor_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCallDeliveryGrantV1 {
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
    /// Independent review fix v5 (section 9/11): mirrors dserver's own field
    /// exactly -- an opaque bare-hex SHA-256 digest reference to the exact
    /// governed source `DCodeInvocationEvidenceV1` that produced this grant's
    /// emit. Used by `dcall::evidence` to prove `dcall.source_dcode_evidence_sha256
    /// == grant.source_dcode_evidence_sha256 == recomputed digest(source
    /// evidence)`.
    pub source_dcode_evidence_sha256: String,
    pub route_kind: String,
    #[serde(default)]
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub endpoint_revision: Option<u64>,
    #[serde(default)]
    pub endpoint_descriptor_digest: Option<String>,
    pub issued_at: String,
    pub expires_at: String,
}

/// Mirrors `dserver::core::dcall::SourceExecutionBindingV1` exactly
/// (independent review fix v4, K2A) -- derived by `dcall::delivery`
/// EXCLUSIVELY from a real source `DCodeInvocationEvidenceV1`, never
/// caller-invented. Sent as `RequestGrantPayload.source_execution`; DServer
/// rejects issuance (`dcall_source_execution_stale`) if it no longer
/// matches current authority.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SourceExecutionBindingV1 {
    pub service_id: String,
    pub node_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub dgraph_digest: String,
    pub dmap_id: String,
    pub dmap_revision: u64,
    pub dmap_digest: String,
    pub dserv_artifact_id: String,
    pub dserv_artifact_digest: String,
    pub dserv_artifact_descriptor_digest: String,
}

pub const ROUTE_KIND_LOCAL: &str = "local";
pub const ROUTE_KIND_REMOTE: &str = "remote";
/// Mirrors `dserver::core::dcall_endpoint::TRANSPORT_HTTP` -- the only
/// v0.1-supported D-Call transport.
pub const TRANSPORT_HTTP: &str = "http";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGrantResponse {
    pub grant: DCallDeliveryGrantV1,
    pub token: String,
}
