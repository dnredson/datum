//! Canonical D-Call HTTP surface (ADR-0022, Phase 112): D-Node transport
//! endpoint declaration, delivery grant issuance, and grant redemption.
//! Lock order, extended from the established `DDEPLOY_TRANSACTION ->
//! DNODE_TRANSACTION -> DCODE_TRANSACTION` chain (`api::dcode`'s own doc
//! comment is the authoritative statement of that order) with
//! `DCALL_TRANSACTION` as the new innermost lock.
//!
//! Independent review fix v3 (J7): an earlier version of this comment
//! incorrectly claimed this file "never acquires any of the other three"
//! locks -- `post_request_grant` and `post_redeem_grant` both correctly
//! acquire the FULL chain, in the documented order, before touching D-Call
//! state (`DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DCODE_TRANSACTION ->
//! DCALL_TRANSACTION`); only the transport-endpoint declaration/read handlers
//! acquire `DCALL_TRANSACTION` alone. The claim that was actually true (and
//! remains true) is narrower: no handler in this file ever acquires any lock
//! in the REVERSE of that order, so there is no reverse-acquisition path
//! anywhere in the crate. No handler in this file holds any lock across
//! network I/O: grant issuance/redemption are pure local computation +
//! persistence; the actual cross-D-Node HTTP delivery happens entirely
//! OUTSIDE these handlers, in DATUM's `dcall::delivery`, using the grant
//! after this endpoint has already released its locks and returned
//! (`dcall-lock-order-audit.txt`).

use std::sync::{Mutex, OnceLock};

use axum::{extract::Json, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::dcall::{
    check_source_execution_binding, derive_dcall_route, issue_grant,
    load_state as load_grant_state, persist_state as persist_grant_state, redeem_grant,
    DCallDeliveryGrantV1, DCallGrantStateV1, RedeemError, SourceExecutionBindingV1,
};
use crate::core::dcall_endpoint::{
    declare_endpoint, load_state as load_endpoint_state, persist_state as persist_endpoint_state,
    DCallEndpointRegistryStateV1, DNodeTransportEndpointV1, DeclareEndpointError,
};
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

static DCALL_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

/// `pub(crate)`, innermost in the established lock chain (see module doc).
pub(crate) fn dcall_transaction() -> &'static Mutex<()> {
    DCALL_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn db_path() -> std::path::PathBuf {
    default_control_state_db_path()
}

fn bad_request(message: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("dcall transaction lock poisoned")
}

fn finding_error(status: StatusCode, finding: CanonicalValidationFinding) -> ApiError {
    (
        status,
        JsonResponse(json!({"error": finding.code, "message": finding.message})),
    )
}

// ---------------------------------------------------------- Endpoint registry

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclareEndpointRequest {
    pub base_url: String,
}

/// `PUT /api/v1/dcall/endpoints/:node_id/:transport`. `node_id` MUST
/// already be a declared structural D-Node (`dnode_registry`) -- an
/// endpoint can never be declared ahead of, or independently from,
/// structural D-Node authority.
pub async fn put_dcall_endpoint(
    axum::extract::Path((node_id, transport)): axum::extract::Path<(String, String)>,
    Json(request): Json<DeclareEndpointRequest>,
) -> Result<JsonResponse<DNodeTransportEndpointV1>, ApiError> {
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dcall_transaction = dcall_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let dnode_registry_state =
        crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let mut endpoint_state = load_endpoint_state(&db_path).map_err(internal_error)?;

    let endpoint = declare_endpoint(
        &mut endpoint_state,
        &dnode_registry_state,
        &node_id,
        &transport,
        &request.base_url,
    )
    .map_err(|error| match error {
        DeclareEndpointError::NodeUnknown(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "dcall_endpoint_node_unknown", "message": message})),
        ),
        DeclareEndpointError::Invalid(finding) => {
            finding_error(StatusCode::UNPROCESSABLE_ENTITY, finding)
        }
    })?;

    persist_endpoint_state(&db_path, &endpoint_state).map_err(internal_error)?;
    Ok(JsonResponse(endpoint))
}

pub async fn get_dcall_endpoint(
    axum::extract::Path((node_id, transport)): axum::extract::Path<(String, String)>,
) -> Result<JsonResponse<DNodeTransportEndpointV1>, ApiError> {
    let state = load_endpoint_state(&db_path()).map_err(internal_error)?;
    state.get(&node_id, &transport).cloned().map(JsonResponse).ok_or_else(|| {
        (StatusCode::NOT_FOUND, JsonResponse(json!({"error": format!("no D-Call endpoint declared for node '{node_id}' transport '{transport}'")})))
    })
}

/// `GET /api/v1/dcall/calls/:project_id/:application_id/:source_service_id`
/// -- lists the `call_id`s (deterministically ordered, section 14) of every
/// D-Call edge sourced from `source_service_id` in the application's
/// CURRENT active D-Graph. This is how `DATUM::dcall::delivery::dispatch_validated_emit`
/// (the D-Engine-facing seam) discovers WHICH calls to attempt for one
/// validated emit -- it never guesses or hardcodes a call_id, and it never
/// receives a destination directly; each discovered call_id is still
/// independently routed/authorized/granted via `derive_dcall_route` before
/// anything is delivered.
pub async fn get_dcalls_for_source_service(
    axum::extract::Path((project_id, application_id, source_service_id)): axum::extract::Path<(
        String,
        String,
        String,
    )>,
) -> Result<JsonResponse<Vec<String>>, ApiError> {
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path()).map_err(internal_error)?;
    let active = ddeploy_state
        .active_by_application
        .get(&crate::core::ddeploy::application_key(
            &project_id,
            &application_id,
        ))
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(
                    json!({"error": "dcall_not_found", "message": "no active accepted D-Map"}),
                ),
            )
        })?;
    let proposal = ddeploy_state
        .proposals
        .get(&active.proposal_id)
        .ok_or_else(|| internal_error("active proposal record missing"))?;
    let call_ids: Vec<String> =
        crate::core::dcall::resolve_dcalls_for_emit(&proposal.dgraph_snapshot, &source_service_id)
        .into_iter()
        .map(|call| call.call_id.clone())
        .collect();
    Ok(JsonResponse(call_ids))
}

// ---------------------------------------------------------------- Grants

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestGrantRequest {
    pub project_id: String,
    pub application_id: String,
    pub call_id: String,
    /// The CALLER's own resolved local D-Node identity (exactly like
    /// `dcode::authorize`'s `node_id`) -- this is the SOURCE node, never
    /// the destination. There is deliberately NO
    /// `destination_service`/`destination_node`/`destination_port`/
    /// `destination_url` field anywhere on this struct
    /// (`#[serde(deny_unknown_fields)]` rejects one if a caller tries) --
    /// see `dcall-route-authority-audit.txt`'s source-guard test.
    pub source_node_id: String,
    pub source_output_port: String,
    pub message_id: String,
    pub correlation_id: String,
    pub payload_sha256: String,
    pub payload_size: u64,
    /// Independent review fix v4 (K2A): the authority the real source
    /// D-Code invocation that produced this emit actually executed under,
    /// derived by the DATUM dispatcher EXCLUSIVELY from that invocation's
    /// own evidence -- never independently caller-invented. Checked for
    /// exact equality against the freshly-derived CURRENT source authority
    /// before any grant is issued (`dcall_source_execution_stale` on
    /// mismatch) -- see `source-execution-authority-binding-audit.txt`.
    pub source_execution: SourceExecutionBindingV1,
    /// Independent review fix v5 (section 9/11): an opaque bare-hex SHA-256
    /// digest reference to the exact governed source `DCodeInvocationEvidenceV1`
    /// that produced this emit -- DServer treats this as an opaque digest to
    /// validate for shape and persist/echo back, never something it
    /// independently reconstructs (it does not possess the full source
    /// evidence record). See `source-dcode-evidence-binding-audit.txt`.
    pub source_dcode_evidence_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct RequestGrantResponse {
    pub grant: DCallDeliveryGrantV1,
    /// The raw bearer token, returned EXACTLY ONCE, here. DServer never
    /// returns it again (only its hash is retained server-side).
    pub token: String,
}

/// `POST /api/v1/dcall/grants`. Derives the D-Call route exclusively from
/// current canonical authority, then issues a short-lived, single-use
/// grant. Never mutates D-Map/placement; never accepts a proposal; issues
/// no host mutation of any kind.
pub async fn post_request_grant(
    Json(request): Json<RequestGrantRequest>,
) -> Result<JsonResponse<RequestGrantResponse>, ApiError> {
    for (field, value) in [
        ("project_id", &request.project_id),
        ("application_id", &request.application_id),
        ("call_id", &request.call_id),
        ("source_node_id", &request.source_node_id),
        ("source_output_port", &request.source_output_port),
        ("message_id", &request.message_id),
        ("correlation_id", &request.correlation_id),
        ("payload_sha256", &request.payload_sha256),
    ] {
        if value.trim().is_empty() {
            return Err(bad_request(format!("{field} must be a non-empty string")));
        }
    }
    // Independent review fix v3 (J3): `payload_sha256` was previously
    // validated only as non-empty, so a malformed value (e.g. "abc") could
    // be persisted successfully here and only fail the ENTIRE grant-state
    // domain's reload validator on restart -- an input-to-persistent-state
    // consistency defect. Reject a malformed shape BEFORE any grant is ever
    // issued, using the SAME shape rule `validate_persisted_dcall_grant_state_integrity`
    // enforces on reload, so a value that would fail one can never have
    // passed the other.
    if !crate::core::dcall::is_valid_sha256_hex(&request.payload_sha256) {
        return Err(bad_request(
            "payload_sha256 must be exactly 64 lowercase hex characters",
        ));
    }
    // Independent review fix v5: reject a malformed source-evidence digest
    // reference BEFORE any grant is ever issued, using the exact same shape
    // rule the persisted-state reload validator enforces.
    if !crate::core::dcall::is_valid_sha256_hex(&request.source_dcode_evidence_sha256) {
        return Err(bad_request(
            "source_dcode_evidence_sha256 must be exactly 64 lowercase hex characters",
        ));
    }

    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dcode_transaction = crate::api::dcode::dcode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dcall_transaction = dcall_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state =
        crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let dcode_registry_state =
        crate::core::dcode_registry::load_state(&db_path).map_err(internal_error)?;
    let endpoint_state = load_endpoint_state(&db_path).map_err(internal_error)?;
    let mut grant_state = load_grant_state(&db_path).map_err(internal_error)?;

    let route = derive_dcall_route(
        &ddeploy_state,
        &dnode_registry_state,
        &dcode_registry_state,
        &request.project_id,
        &request.application_id,
        &request.call_id,
        // `source_service_id` is derived from the D-Call edge itself once
        // `call_id` resolves -- the request never names it directly, only
        // the source NODE it claims to run on.
        &{
            // Resolve the DGraph's own source_service_id for this call_id
            // before calling `derive_dcall_route` proper, purely so a
            // caller cannot claim a call_id that does not actually belong
            // to the node it says it's calling from -- `derive_dcall_route`
            // re-validates this exact binding internally regardless.
            // Phase 113A closure pass 2: sourced from the shared v1/v2
            // authority accessor, not `active_by_application` directly.
            let active = crate::core::ddeploy::active_placement_authority(
                &ddeploy_state,
                &request.project_id,
                &request.application_id,
            )
            .map_err(|error| {
                finding_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    CanonicalValidationFinding {
                        code: "dcall_not_found",
                        message: error,
                    },
                )
            })?
            .ok_or_else(|| {
                finding_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    CanonicalValidationFinding {
                        code: "dcall_not_found",
                        message: "no active accepted D-Map".to_string(),
                    },
                )
            })?;
            let dgraph_snapshot = active.dgraph_snapshot.as_ref().ok_or_else(|| {
                finding_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    CanonicalValidationFinding {
                        code: "dcall_not_found",
                        message: "active application authority carries no D-Graph snapshot"
                            .to_string(),
                    },
                )
            })?;
            dgraph_snapshot
                .calls
                .iter()
                .find(|call| call.call_id == request.call_id)
                .map(|call| call.source_service_id.clone())
                .ok_or_else(|| {
                    finding_error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        CanonicalValidationFinding {
                            code: "dcall_not_found",
                            message: format!(
                                "no D-Call '{}' exists in the active D-Graph",
                                request.call_id
                            ),
                        },
                    )
                })?
        },
        &request.source_node_id,
        &request.source_output_port,
    )
    .map_err(|finding| finding_error(StatusCode::UNPROCESSABLE_ENTITY, finding))?;

    // Phase 113A (ADR-0023) realization boundary: this is the ONE
    // authoritative production dispatch boundary (route derivation succeeds
    // under an active DMap/2, per `derive_dcall_route`'s own doc comment --
    // DGraph/DMap authority is unaffected -- but no grant may ever be issued
    // for a D-Call governed under DMap/2, since Phase 113A implements no
    // runtime transport for any canonical realization). Checked HERE,
    // BEFORE endpoint lookup, `check_source_execution_binding`, the
    // message-limit checks, and `issue_grant` -- no delivery side effect
    // (grant/token/endpoint lookup/network send) has occurred yet, and no
    // duplicate copy of this check exists anywhere else (client/listener
    // code never re-derives a route). A DMap/1 active authority is entirely
    // unaffected (`v2_dcall_realization` is `None`).
    if let Some(realization) = &route.v2_dcall_realization {
        return Err(finding_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            CanonicalValidationFinding {
                code: "dcall_realization_transport_not_implemented",
                message: format!(
                    "D-Call '{}' is governed under the active DMap/2 with realization kind '{}', but no runtime transport is implemented for it yet -- no grant can be issued yet",
                    route.call_id, realization.realization_kind
                ),
            },
        ));
    }

    // K2B: the authority the source invocation actually executed under
    // (caller-supplied, but derived exclusively by DATUM from that real
    // invocation's own evidence) must exactly match the authority just
    // freshly derived above as CURRENT -- otherwise a legitimate topology
    // change between the source's real execution and this grant request
    // could silently route an OLD emit under NEW canonical authority. This
    // must run, and must succeed, BEFORE any grant is inserted into state.
    check_source_execution_binding(&route, &request.source_execution)
        .map_err(|finding| finding_error(StatusCode::UNPROCESSABLE_ENTITY, finding))?;

    // I5: pre-grant message-size validation. This is a conservative bound on
    // the RAW payload only -- it does not, and cannot without duplicating
    // the full ADR-0003 envelope serializer here, prove the final
    // base64-wrapped JSON envelope itself fits under `max_message_bytes`;
    // Phase-111's runtime remains the exact, final enforcement at actual
    // invocation time. What this check DOES guarantee: an oversized payload
    // is rejected BEFORE a grant is ever inserted into state, so a one-time
    // grant is never wasted on a delivery that could never have succeeded
    // (`dcall-message-limit-audit.txt`).
    if let Some(destination_artifact) =
        dcode_registry_state.get(&request.application_id, &route.destination.service_id)
    {
        if request.payload_size > u64::from(destination_artifact.limits.max_message_bytes) {
            return Err(finding_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                CanonicalValidationFinding {
                    code: "dcall_payload_exceeds_destination_message_limit",
                    message: format!(
                        "payload_size {} exceeds destination artifact's max_message_bytes {} (raw-payload bound, not the final serialized envelope)",
                        request.payload_size, destination_artifact.limits.max_message_bytes
                    ),
                },
            ));
        }
    }
    if let Some(source_artifact) =
        dcode_registry_state.get(&request.application_id, &route.source.service_id)
    {
        if request.payload_size > u64::from(source_artifact.limits.max_message_bytes) {
            return Err(finding_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                CanonicalValidationFinding {
                    code: "dcall_payload_exceeds_source_message_limit",
                    message: format!(
                        "payload_size {} exceeds source artifact's own max_message_bytes {} (raw-payload bound)",
                        request.payload_size, source_artifact.limits.max_message_bytes
                    ),
                },
            ));
        }
    }

    let endpoint = if route.route_kind == crate::core::dcall::ROUTE_KIND_REMOTE {
        Some(
            endpoint_state
                .get(
                    &route.destination.node_id,
                    crate::core::dcall_endpoint::TRANSPORT_HTTP,
                )
                .cloned()
                .ok_or_else(|| {
                    finding_error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        CanonicalValidationFinding {
                            code: "dcall_destination_endpoint_missing",
                            message: format!(
                                "no HTTP transport endpoint is declared for destination node '{}'",
                                route.destination.node_id
                            ),
                        },
                    )
                })?,
        )
    } else {
        None
    };

    let issued = issue_grant(
        &mut grant_state,
        route,
        endpoint.as_ref(),
        request.message_id,
        request.correlation_id,
        request.payload_sha256,
        request.payload_size,
        request.source_dcode_evidence_sha256,
        Utc::now(),
    )
    .map_err(|finding| finding_error(StatusCode::UNPROCESSABLE_ENTITY, finding))?;

    persist_grant_state(&db_path, &grant_state).map_err(internal_error)?;

    Ok(JsonResponse(RequestGrantResponse {
        grant: issued.grant,
        token: issued.token,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemGrantRequest {
    pub token: String,
    pub destination_node_id: String,
    pub payload_sha256: String,
    pub payload_size: u64,
}

/// `POST /api/v1/dcall/grants/:grant_id/redeem`. Atomic, one-time,
/// fail-closed. Re-checks current canonical authority (source-of-record
/// D-Map/D-Graph/artifact/endpoint) before marking the grant redeemed --
/// see `core::dcall::redeem_grant`'s doc for the exact guarantee.
pub async fn post_redeem_grant(
    axum::extract::Path(grant_id): axum::extract::Path<String>,
    Json(request): Json<RedeemGrantRequest>,
) -> Result<JsonResponse<DCallDeliveryGrantV1>, ApiError> {
    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dcode_transaction = crate::api::dcode::dcode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dcall_transaction = dcall_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state =
        crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let dcode_registry_state =
        crate::core::dcode_registry::load_state(&db_path).map_err(internal_error)?;
    let endpoint_state = load_endpoint_state(&db_path).map_err(internal_error)?;
    let mut grant_state = load_grant_state(&db_path).map_err(internal_error)?;

    let grant = redeem_grant(
        &mut grant_state,
        &ddeploy_state,
        &dnode_registry_state,
        &dcode_registry_state,
        &endpoint_state,
        &grant_id,
        &request.token,
        &request.destination_node_id,
        &request.payload_sha256,
        request.payload_size,
        Utc::now(),
    )
    .map_err(|error| match error {
        RedeemError::NotFound => (
            StatusCode::NOT_FOUND,
            JsonResponse(json!({"error": "dcall_grant_not_found"})),
        ),
        RedeemError::TokenInvalid => (
            StatusCode::FORBIDDEN,
            JsonResponse(json!({"error": "dcall_grant_token_invalid"})),
        ),
        RedeemError::Expired => (
            StatusCode::GONE,
            JsonResponse(json!({"error": "dcall_grant_expired"})),
        ),
        RedeemError::AlreadyRedeemed => (
            StatusCode::CONFLICT,
            JsonResponse(json!({"error": "dcall_grant_already_redeemed"})),
        ),
        RedeemError::DestinationNodeMismatch => (
            StatusCode::FORBIDDEN,
            JsonResponse(json!({"error": "dcall_grant_wrong_destination_node"})),
        ),
        RedeemError::PayloadMismatch => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "dcall_payload_digest_mismatch"})),
        ),
        RedeemError::StaleAuthority(finding) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": finding.code, "message": finding.message})),
        ),
    })?;

    persist_grant_state(&db_path, &grant_state).map_err(internal_error)?;
    Ok(JsonResponse(grant))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section 23 source guard: a caller MUST NOT be able to name a
    /// destination service/node/port/URL as authoritative routing input.
    /// `#[serde(deny_unknown_fields)]` on `RequestGrantRequest` makes any
    /// attempt to smuggle one in a hard parse failure, not a silently
    /// ignored field.
    #[test]
    fn request_grant_payload_rejects_destination_fields() {
        for field in [
            "destination_service_id",
            "destination_node_id",
            "destination_port",
            "destination_url",
        ] {
            let mut value = json!({
                "project_id": "proj:lab", "application_id": "app:a", "call_id": "c1",
                "source_node_id": "fog-01", "source_output_port": "out",
                "message_id": "m1", "correlation_id": "c1",
                "payload_sha256": "a".repeat(64), "payload_size": 4,
            });
            value
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), json!("caller-chosen"));
            let result: Result<RequestGrantRequest, _> = serde_json::from_value(value);
            assert!(
                result.is_err(),
                "expected {field} to be rejected by deny_unknown_fields"
            );
        }
    }

    #[test]
    fn request_grant_payload_accepts_the_legitimate_minimal_shape() {
        let value = json!({
            "project_id": "proj:lab", "application_id": "app:a", "call_id": "c1",
            "source_node_id": "fog-01", "source_output_port": "out",
            "message_id": "m1", "correlation_id": "c1",
            "payload_sha256": "a".repeat(64), "payload_size": 4,
            "source_execution": {
                "service_id": "svc:a", "node_id": "fog-01",
                "dgraph_id": "dgraph:a", "dgraph_revision": 1, "dgraph_digest": "sha256:a".to_string() + &"a".repeat(63),
                "dmap_id": "dmap:a", "dmap_revision": 1, "dmap_digest": "sha256:a".to_string() + &"a".repeat(63),
                "dserv_artifact_id": "artifact:a", "dserv_artifact_digest": "a".repeat(64),
                "dserv_artifact_descriptor_digest": "sha256:a".to_string() + &"a".repeat(63),
            },
            "source_dcode_evidence_sha256": "a".repeat(64),
        });
        let result: Result<RequestGrantRequest, _> = serde_json::from_value(value);
        assert!(result.is_ok());
    }
}
