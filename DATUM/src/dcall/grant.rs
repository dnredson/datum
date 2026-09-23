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

/// Phase 113B2 (ADR-0023): mirrors
/// `dserver::core::dcall::DCallRealizationTransportAuthorityV1` exactly.
/// `Some(..)` only for a grant governing a DMap/2 `via_diot_pubsub` D-Call;
/// `None` for every DMap/1 grant. Deliberately carries NO `broker_uri` (see
/// the dserver-side type's own doc comment for the exact rationale) -- a
/// future MQTT transport slice (113B3) resolves the current broker URI by
/// looking the binding up again, never by trusting a value cached here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCallRealizationTransportAuthorityV1 {
    pub realization_kind: String,
    pub diot_placement_id: String,
    pub interface: String,
    pub channel: String,
    pub dforward_id: String,
    pub dforward_revision: u64,
    pub dforward_digest: String,
    pub binding_id: String,
    pub binding_revision: u64,
    pub descriptor_digest: String,
    pub transport: String,
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
    /// Phase 113B2: mirrors dserver's own field exactly. `#[serde(default)]`
    /// so a DMap/1 response (which never carries this field) still
    /// deserializes as `None` -- never fabricated.
    #[serde(default)]
    pub v2_dcall_transport_authority: Option<DCallRealizationTransportAuthorityV1>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_grant_json() -> serde_json::Value {
        serde_json::json!({
            "grant_id": "grant-1",
            "project_id": "proj:lab",
            "application_id": "app:a",
            "call_id": "c1",
            "dgraph_ref": {"id": "dgraph:a", "revision": 1, "digest": format!("sha256:{}", "a".repeat(64))},
            "dmap_ref": {"id": "dmap:a", "revision": 1, "digest": format!("sha256:{}", "a".repeat(64))},
            "source": {
                "service_id": "svc:src", "port": "out", "node_id": "fog-01",
                "dserv_artifact_id": "artifact:src", "dserv_artifact_digest": "b".repeat(64),
                "dserv_artifact_descriptor_digest": format!("sha256:{}", "b".repeat(64)),
            },
            "destination": {
                "service_id": "svc:dst", "port": "in", "node_id": "fog-01",
                "dserv_artifact_id": "artifact:dst", "dserv_artifact_digest": "c".repeat(64),
                "dserv_artifact_descriptor_digest": format!("sha256:{}", "c".repeat(64)),
            },
            "message_id": "m1",
            "correlation_id": "cr1",
            "payload_sha256": "d".repeat(64),
            "payload_size": 4,
            "source_dcode_evidence_sha256": "e".repeat(64),
            "route_kind": "local",
            "issued_at": "2026-01-01T00:00:00Z",
            "expires_at": "2026-01-01T00:00:30Z",
        })
    }

    /// Phase 113B2.1 section 4: a historical DMap/1 grant JSON that
    /// genuinely OMITS `v2_dcall_transport_authority` entirely (never
    /// merely sets it to `null`) must still deserialize, with the field
    /// resolving to `None` via `#[serde(default)]` -- never fabricated.
    /// Mirrors dserver's own
    /// `persisted_historical_dmap1_grant_without_the_v2_field_still_reloads`.
    #[test]
    fn historical_dmap1_grant_json_without_the_v2_field_still_deserializes() {
        let value = minimal_grant_json();
        let text = serde_json::to_string(&value).unwrap();
        assert!(!text.contains("v2_dcall_transport_authority"));
        let grant: DCallDeliveryGrantV1 = serde_json::from_str(&text)
            .expect("a historical grant missing this field entirely must still deserialize");
        assert_eq!(grant.v2_dcall_transport_authority, None);
    }

    /// A DMap/2 grant JSON that DOES carry the typed transport authority
    /// deserializes it exactly, field-for-field.
    #[test]
    fn dmap2_grant_json_with_the_v2_field_deserializes_exactly() {
        let mut value = minimal_grant_json();
        value["v2_dcall_transport_authority"] = serde_json::json!({
            "realization_kind": "via_diot_pubsub",
            "diot_placement_id": "diotp:broker",
            "interface": "sensor-bus",
            "channel": "pipeline/sensor-to-decoder",
            "dforward_id": "dforward:pipeline",
            "dforward_revision": 1,
            "dforward_digest": format!("sha256:{}", "f".repeat(64)),
            "binding_id": "binding:diotp-broker",
            "binding_revision": 1,
            "descriptor_digest": format!("sha256:{}", "0".repeat(64)),
            "transport": "mqtt",
        });
        let grant: DCallDeliveryGrantV1 =
            serde_json::from_value(value).expect("a DMap/2 grant must deserialize");
        let authority = grant
            .v2_dcall_transport_authority
            .expect("the typed transport authority must be present");
        assert_eq!(authority.realization_kind, "via_diot_pubsub");
        assert_eq!(authority.binding_id, "binding:diotp-broker");
    }
}
