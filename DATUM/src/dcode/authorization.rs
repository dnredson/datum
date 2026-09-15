//! Mirrors DServer's `core::dcode_authorization::DCodeInvocationAuthorizationV1`
//! wire shape exactly (independently, per this repository's established
//! no-shared-crate discipline, ADR-0007) -- SmartSentinel deserializes this
//! JSON directly from a live `POST /api/v1/dcode/authorize` response (the
//! governed path, `dcode::client`) or, for internal/test-only use, from a
//! local file.
//!
//! Independent review v2 (R2B) correction: this mirror was previously STALE
//! relative to dserver's R2A fix -- it had only `dserv_artifact_digest` and
//! was missing `dserv_artifact_descriptor_digest` entirely. Because this
//! struct did not `deny_unknown_fields`, deserializing a real authorization
//! from the (already-fixed) `/authorize` endpoint would have SILENTLY
//! DROPPED the new field rather than erroring, so a real invocation could
//! never have actually checked descriptor-digest binding no matter what the
//! server sent. `#[serde(deny_unknown_fields)]` is now added specifically so
//! this class of silent-drift-via-silent-field-drop can never recur
//! unnoticed: any future field dserver adds to the real struct without a
//! matching DATUM-side update now fails deserialization loudly instead of
//! silently ignoring the new data.

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
pub struct DCodeInvocationAuthorizationV1 {
    pub authorization_id: String,
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
    pub node_id: String,
    pub dmap_ref: CanonicalContractReferenceV1,
    pub dgraph_ref: CanonicalContractReferenceV1,
    pub dserv_artifact_id: String,
    /// D-Code MODULE content digest only (`artifact.dcode.sha256`).
    pub dserv_artifact_digest: String,
    /// Canonical JCS digest of the WHOLE `datum.dserv-artifact/1` descriptor
    /// -- see `dserver::core::dcode_authorization::DCodeInvocationAuthorizationV1`
    /// for why this is deliberately distinct from `dserv_artifact_digest`.
    pub dserv_artifact_descriptor_digest: String,
    pub dnode_abi: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R2B regression: a real authorization JSON shaped exactly like
    /// dserver's current `DCodeInvocationAuthorizationV1` (both digest
    /// fields present) must deserialize with no field silently dropped.
    #[test]
    fn real_shaped_authorization_json_round_trips_both_digest_fields() {
        let json = serde_json::json!({
            "authorization_id": "dcode-auth:proj:app:svc:node:1",
            "project_id": "proj:lab",
            "application_id": "app:sensor-pipeline",
            "service_id": "decode",
            "node_id": "fog-01",
            "dmap_ref": {"id": "dmap:1:1:app", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
            "dgraph_ref": {"id": "dgraph:sensor-pipeline", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
            "dserv_artifact_id": "dserv:sensor-pipeline:decode",
            "dserv_artifact_digest": "a".repeat(64),
            "dserv_artifact_descriptor_digest": format!("sha256:{}", "3".repeat(64)),
            "dnode_abi": "datum-dnode/0",
        });
        let authorization: DCodeInvocationAuthorizationV1 =
            serde_json::from_value(json).expect("real-shaped authorization JSON must deserialize cleanly");
        assert_eq!(authorization.dserv_artifact_digest, "a".repeat(64));
        assert_eq!(authorization.dserv_artifact_descriptor_digest, format!("sha256:{}", "3".repeat(64)));
    }

    /// An authorization missing the descriptor digest field entirely (the
    /// pre-R2B shape) must now fail to deserialize rather than silently
    /// defaulting -- this field carries security-relevant binding data, not
    /// a value with any safe default.
    #[test]
    fn authorization_missing_descriptor_digest_field_fails_to_deserialize() {
        let json = serde_json::json!({
            "authorization_id": "dcode-auth:proj:app:svc:node:1",
            "project_id": "proj:lab",
            "application_id": "app:sensor-pipeline",
            "service_id": "decode",
            "node_id": "fog-01",
            "dmap_ref": {"id": "dmap:1:1:app", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
            "dgraph_ref": {"id": "dgraph:sensor-pipeline", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
            "dserv_artifact_id": "dserv:sensor-pipeline:decode",
            "dserv_artifact_digest": "a".repeat(64),
            "dnode_abi": "datum-dnode/0",
        });
        let result: Result<DCodeInvocationAuthorizationV1, _> = serde_json::from_value(json);
        assert!(result.is_err(), "missing dserv_artifact_descriptor_digest must be a hard deserialization error");
    }
}
