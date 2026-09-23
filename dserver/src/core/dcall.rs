//! Canonical D-Call route derivation + delivery grant (ADR-0022, Phase 112).
//!
//! `derive_dcall_route` is the ROUTE-DERIVATION authority: given a
//! project/application/call identity and the source's own (already
//! D-Code-authorized) service/node/output-port, it derives the destination
//! service/port/node EXCLUSIVELY from the currently active accepted D-Map
//! and canonical D-Graph -- a caller can never select the destination
//! service, port, or node (`dcall-route-authority-audit.txt`). It fully
//! reuses `dcode_authorization::authorize_dcode_invocation` for BOTH the
//! source and destination endpoints, so every Phase-111 authority check
//! (active D-Map, D-Graph membership, placement, D-Node ABI, artifact
//! identity/digest/stage-constraint) applies identically to D-Call
//! delivery -- this module adds no parallel/weaker authority path.
//!
//! `datum.dgraph/1`'s `CanonicalDCallV1` (`call_id`/`source_service_id`/
//! `target_service_id`) carries NO port field (see
//! `dcall-existing-contract-audit.txt`) -- ports are exclusively a
//! `datum.dserv-artifact/1` (ADR-0021) concept. The destination input port
//! is therefore resolved, not invented: it is the destination artifact's
//! declared input port whose `payload_schema` matches the source's
//! emitting output port's `payload_schema`, and v0.1 REQUIRES this match to
//! be unique (ambiguous or absent match fails closed) -- see
//! `dcall-artifact-port-audit.txt` for the full rationale and the
//! alternative designs considered.
//!
//! `DCallDeliveryGrantV1` is the load-bearing security primitive
//! (`dcall-grant-authority-audit.txt`): short-lived, single-use, binding
//! the full authority chain a delivery depends on. Only `SHA-256(token)` is
//! ever persisted (`dcall-grant-replay-audit.txt`); redemption is atomic,
//! one-time, and durable across restart.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::dcall_endpoint::{
    DCallEndpointRegistryStateV1, DNodeTransportEndpointV1, TRANSPORT_HTTP,
};
use crate::core::dcall_mqtt_topic::request_topic as compute_mqtt_request_topic;
use crate::core::dcode_authorization::authorize_dcode_invocation;
use crate::core::dcode_registry::DCodeArtifactRegistryStateV1;
use crate::core::ddeploy::DDeployStateV1;
use crate::core::diot_runtime_binding::{
    verify_runtime_binding_still_current, DIoTRuntimeBindingRegistryStateV1,
};
use crate::core::dnode_registry::DNodeRegistryStateV1;
use crate::core::jcs::is_canonical_digest_format;
use crate::core::sha256::sha256_hex;

/// Independent review fix v3 (J3A): the bare (unprefixed) 64-lowercase-hex
/// SHA-256 representation `payload_sha256`/`token_hash`/the module-content
/// `dserv_artifact_digest` all use throughout this module -- distinct from
/// `jcs::is_canonical_digest_format`'s `sha256:<64 lowercase hex>` PREFIXED
/// form (used for `dgraph_ref`/`dmap_ref`/`dserv_artifact_descriptor_digest`/
/// endpoint `descriptor_digest`). Used to validate BOTH a caller's
/// request-grant input (before `issue_grant` ever runs) and already-persisted
/// state on reload, so a value that fails one check can never have passed
/// the other. An earlier version of the two call sites this replaces used
/// only `is_ascii_hexdigit()` (which also accepts uppercase A-F) while their
/// error messages claimed "lowercase" -- this closes that inconsistency too.
pub(crate) fn is_valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
use crate::storage::control_state::{
    load_domain_json, persist_domain_json, DOMAIN_DCALL_GRANT_STATE,
};

pub const DCALL_GRANT_STATE_VERSION: &str = "dcall_grant_state_v1";
/// v0.1 fixed grant lifetime. A grant is meant to be redeemed within one
/// control-plane round trip immediately followed by delivery -- not held
/// open. Not currently configurable per artifact/call; a fixed
/// conservative value is the smallest design consistent with "short-lived".
pub const GRANT_TTL_SECONDS: i64 = 30;

pub const ROUTE_KIND_LOCAL: &str = "local";
pub const ROUTE_KIND_REMOTE: &str = "remote";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCallRouteEndpointV1 {
    pub service_id: String,
    pub port: String,
    pub node_id: String,
    pub dserv_artifact_id: String,
    pub dserv_artifact_digest: String,
    pub dserv_artifact_descriptor_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCallRouteV1 {
    pub project_id: String,
    pub application_id: String,
    pub call_id: String,
    pub dgraph_ref: CanonicalContractReferenceV1,
    pub dmap_ref: CanonicalContractReferenceV1,
    pub source: DCallRouteEndpointV1,
    pub destination: DCallRouteEndpointV1,
    /// `"local"` (`ROUTE_KIND_LOCAL`) or `"remote"` (`ROUTE_KIND_REMOTE`) --
    /// derived, never caller-supplied.
    pub route_kind: String,
    /// Phase 113A/113B1 (ADR-0023): `Some(..)` when this D-Call is governed
    /// under an ACTIVE DMap/2, `None` under DMap/1. A route can always be
    /// DERIVED under DMap/2 (DGraph/DMap authority is unaffected -- see
    /// `derive_dcall_route`'s doc comment), but `issue_grant` refuses to
    /// issue a grant when this is `Some`, since no runtime transport exists
    /// for any realization yet (the MQTT wire path is a later closure).
    /// Never caller-supplied.
    pub v2_dcall_realization: Option<DCallRealizationAuthorityV1>,
}

/// Phase 113B1: the exact D-Call realization authority for one D-Call under
/// an active DMap/2, sourced EXCLUSIVELY from `active_placement_authority()`
/// (the `DCallRealizationV2` entry matching this route's `call_id`, plus the
/// governing DForward's own identity) -- never caller input. `realization_kind`
/// is always `"via_diot_pubsub"` in v0.1 (the only value
/// `CanonicalDMapV2::validate` ever accepts). `dforward_id`/`dforward_revision`/
/// `dforward_digest` are carried so a consumer (the DIoT runtime-binding
/// registry) can detect staleness against the DForward that was active at
/// registration time without a second lookup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCallRealizationAuthorityV1 {
    pub realization_kind: String,
    pub diot_placement_id: String,
    pub interface: String,
    pub channel: String,
    pub dforward_id: String,
    pub dforward_revision: u64,
    pub dforward_digest: String,
}

/// Phase 113B2 (ADR-0023): the exact server-derived transport authority a
/// `DCallDeliveryGrantV1` binds when it governs a DMap/2 `via_diot_pubsub`
/// D-Call -- `None` for every DMap/1 grant (unchanged, section 2/4). Binds
/// BOTH halves of what B2 requires: the accepted D-Call realization itself
/// (`realization_kind`/`diot_placement_id`/`interface`/`channel`/
/// `dforward_id`/`dforward_revision`/`dforward_digest`, copied verbatim from
/// the route's own `DCallRealizationAuthorityV1` -- never re-derived
/// independently, so the two can never silently disagree) AND the governed
/// DIoT runtime-binding identity current at issuance
/// (`binding_id`/`binding_revision`/`descriptor_digest`, from
/// `diot_runtime_binding::DIoTRuntimeBindingV1`). `transport` is carried
/// (always `"mqtt"` in v0.1) so a future consumer never has to guess it from
/// the presence of this struct alone.
///
/// Section 5 (`broker_uri` inclusion decision): deliberately EXCLUDED.
/// `binding_id`/`binding_revision`/`descriptor_digest` are what make this
/// authority load-bearing (they let a consumer prove, via
/// `diot_runtime_binding::verify_runtime_binding_still_current`, that the
/// binding is STILL current); a bare `broker_uri` string would be
/// non-authoritative duplicate data that could only ever go stale
/// independently of those fields, and Phase 113B2 implements no transport
/// that would consume it anyway. A future MQTT transport slice (113B3) reads
/// the current `broker_uri` by looking the binding up again by
/// `(project_id, application_id, diot_placement_id)` -- the grant's own
/// `project_id`/`application_id` plus this struct's `diot_placement_id`
/// already carry everything needed to do that lookup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

/// Independent review fix v4 (K2A): the authority a real source D-Code
/// invocation actually executed under, derived EXCLUSIVELY by the DATUM
/// dispatcher from that invocation's own `DCodeInvocationEvidenceV1`
/// (never as independent caller-supplied fields). Threaded into
/// `RequestGrantRequest.source_execution` so grant issuance (`K2B`) can
/// prove `source execution authority == authority at issuance`, closing
/// the race where a legitimate topology change between a source's real
/// execution and a later grant request for its emit could otherwise
/// silently route an OLD emit under NEW canonical authority -- see
/// `source-execution-authority-binding-audit.txt`. This proves AUTHORITY
/// CONTINUITY for the governed DATUM execution path; it is not
/// cryptographic remote attestation against an arbitrary hostile HTTP
/// caller -- DServer still trusts its authenticated control-plane/host
/// boundary (`dcall-grant-authority-continuity-audit.txt`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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

/// Independent review fix v4 (K2B): compares a caller-declared
/// `SourceExecutionBindingV1` against the route's own freshly-derived
/// CURRENT source authority, requiring EXACT equality on every field. Must
/// be called (and must succeed) BEFORE `issue_grant` -- a mismatch means
/// the emit being dispatched was produced under authority that is no
/// longer current, so no grant may be issued for it at all.
pub fn check_source_execution_binding(
    route: &DCallRouteV1,
    binding: &SourceExecutionBindingV1,
) -> Result<(), CanonicalValidationFinding> {
    let mismatch = |field: &str, expected: &str, actual: &str| {
        finding("dcall_source_execution_stale", format!("source execution authority is stale: {field} was '{expected}' at source execution but is now '{actual}' -- create a new source emit under current authority"))
    };
    if binding.service_id != route.source.service_id {
        return Err(mismatch(
            "source service_id",
            &binding.service_id,
            &route.source.service_id,
        ));
    }
    if binding.node_id != route.source.node_id {
        return Err(mismatch(
            "source node_id",
            &binding.node_id,
            &route.source.node_id,
        ));
    }
    if binding.dgraph_id != route.dgraph_ref.id
        || binding.dgraph_revision != route.dgraph_ref.revision
        || binding.dgraph_digest != route.dgraph_ref.digest
    {
        return Err(mismatch(
            "DGraph ref",
            &format!(
                "{}@{}#{}",
                binding.dgraph_id, binding.dgraph_revision, binding.dgraph_digest
            ),
            &format!(
                "{}@{}#{}",
                route.dgraph_ref.id, route.dgraph_ref.revision, route.dgraph_ref.digest
            ),
        ));
    }
    if binding.dmap_id != route.dmap_ref.id
        || binding.dmap_revision != route.dmap_ref.revision
        || binding.dmap_digest != route.dmap_ref.digest
    {
        return Err(mismatch(
            "DMap ref",
            &format!(
                "{}@{}#{}",
                binding.dmap_id, binding.dmap_revision, binding.dmap_digest
            ),
            &format!(
                "{}@{}#{}",
                route.dmap_ref.id, route.dmap_ref.revision, route.dmap_ref.digest
            ),
        ));
    }
    if binding.dserv_artifact_id != route.source.dserv_artifact_id {
        return Err(mismatch(
            "source dserv_artifact_id",
            &binding.dserv_artifact_id,
            &route.source.dserv_artifact_id,
        ));
    }
    if binding.dserv_artifact_digest != route.source.dserv_artifact_digest {
        return Err(mismatch(
            "source module digest",
            &binding.dserv_artifact_digest,
            &route.source.dserv_artifact_digest,
        ));
    }
    if binding.dserv_artifact_descriptor_digest != route.source.dserv_artifact_descriptor_digest {
        return Err(mismatch(
            "source artifact descriptor digest",
            &binding.dserv_artifact_descriptor_digest,
            &route.source.dserv_artifact_descriptor_digest,
        ));
    }
    Ok(())
}

/// Derives one D-Call's operational route, EXCLUSIVELY from currently
/// active canonical authority. `source_node_id` is the CALLER's own
/// resolved local D-Node identity (exactly analogous to
/// `dcode_authorization::authorize_dcode_invocation`'s `node_id` parameter
/// -- proves "the active D-Map places this service on the node claiming to
/// be it", never trusted as proof of physical identity beyond that). The
/// destination service/port/node are 100% DERIVED below; nothing in this
/// function's parameter list lets a caller name them.
#[allow(clippy::too_many_arguments)]
pub fn derive_dcall_route(
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    dcode_registry_state: &DCodeArtifactRegistryStateV1,
    project_id: &str,
    application_id: &str,
    call_id: &str,
    source_service_id: &str,
    source_node_id: &str,
    source_output_port: &str,
) -> Result<DCallRouteV1, CanonicalValidationFinding> {
    // Full Phase-111 authority chain for the SOURCE (this also proves the
    // emitting invocation's own node/service/artifact are still currently
    // authoritative, not merely that they once were).
    let source_auth = authorize_dcode_invocation(
        ddeploy_state,
        dnode_registry_state,
        dcode_registry_state,
        project_id,
        application_id,
        source_service_id,
        source_node_id,
    )?;

    // Phase 113A closure pass 2: sourced EXCLUSIVELY from the shared v1/v2
    // authority accessor -- this function no longer reads
    // `active_by_application`/`proposals` directly, so route derivation is
    // identical whether the active authority is DMap/1 or DMap/2.
    let active = crate::core::ddeploy::active_placement_authority(ddeploy_state, project_id, application_id)
        .map_err(|error| finding("dcall_not_found", error))?
        .ok_or_else(|| finding("dcall_not_found", format!("project '{project_id}' application '{application_id}' has no active accepted D-Map")))?;
    let dgraph_snapshot = active.dgraph_snapshot.as_ref().ok_or_else(|| {
        finding(
            "dcall_not_found",
            "active application authority carries no D-Graph snapshot",
        )
    })?;

    // The DGraph edge is resolved by (call_id, source_service_id) -- NOT by
    // call_id alone -- so a caller cannot claim a real call_id belongs to a
    // service it does not actually source from.
    let call = dgraph_snapshot
        .calls
        .iter()
        .find(|call| call.call_id == call_id && call.source_service_id == source_service_id)
        .ok_or_else(|| {
            finding(
                "dcall_not_found",
                format!("no D-Call '{call_id}' with source_service_id '{source_service_id}' exists in the active D-Graph"),
            )
        })?;
    let destination_service_id = call.target_service_id.clone();

    // Phase 113A/113B1 (ADR-0023) realization boundary: route derivation
    // itself still succeeds under an active DMap/2 (DGraph stays sole
    // semantic authority, DMap/2 stays sole placement authority -- section
    // 7), but if this D-Call is governed under DMap/2 the caller
    // (`issue_grant`, the ONE authoritative production dispatch boundary --
    // see its own doc comment) must refuse to issue a grant for it, since no
    // runtime transport exists for ANY canonical realization yet. `None`
    // here means the active authority is DMap/1 and is entirely unaffected.
    //
    // `CanonicalDMapV2::validate` requires EXACTLY ONE `DCallRealizationV2`
    // per DGraph call (`missing_dcall_realization`), so a validated,
    // currently-accepted DMap/2 can never be missing one for a call_id that
    // its own governing DGraph declares. If this lookup nonetheless finds
    // none, that is persisted-authority corruption/inconsistency -- not a
    // caller error and not a gap to paper over -- so this fails closed
    // rather than fabricating a realization (independent review fix: an
    // earlier version silently invented `"via_diot_pubsub"` here).
    let v2_dcall_realization = match active.v2.as_ref() {
        None => None,
        Some(v2) => {
            let realization = v2
                .dcall_realizations
                .iter()
                .find(|r| r.call_id == call_id)
                .ok_or_else(|| {
                    finding(
                        "dcall_v2_realization_missing",
                        format!(
                            "active DMap/2 governs D-Call '{call_id}' but declares no matching D-Call realization -- this is a persisted-authority inconsistency, not a caller error"
                        ),
                    )
                })?;
            Some(DCallRealizationAuthorityV1 {
                realization_kind: realization.realization_kind.clone(),
                diot_placement_id: realization.diot_placement_id.clone(),
                interface: realization.interface.clone(),
                channel: realization.channel.clone(),
                dforward_id: v2.dforward_id.clone(),
                dforward_revision: v2.dforward_revision,
                dforward_digest: v2.dforward_digest.clone(),
            })
        }
    };

    // Destination node: DERIVED from the active D-Map's own placements,
    // never from any caller input.
    let destination_placement = active
        .placements
        .iter()
        .find(|placement| placement.service_id == destination_service_id)
        .ok_or_else(|| {
            finding(
                "dcall_destination_service_not_placed",
                format!("D-Call destination service '{destination_service_id}' has no placement in the active D-Map"),
            )
        })?;
    let destination_node_id = destination_placement.node_id.clone();

    // Full Phase-111 authority chain for the DESTINATION, using the node ID
    // just derived above (never a caller-supplied one).
    let dest_auth = authorize_dcode_invocation(
        ddeploy_state,
        dnode_registry_state,
        dcode_registry_state,
        project_id,
        application_id,
        &destination_service_id,
        &destination_node_id,
    )?;

    // Port resolution (see module doc): source output port -> matching
    // payload_schema -> unique destination input port.
    let source_artifact = dcode_registry_state
        .get(application_id, source_service_id)
        .ok_or_else(|| {
            finding(
                "dcall_source_port_invalid",
                "source D-Serv artifact is not registered",
            )
        })?;
    let source_port_decl = source_artifact
        .ports
        .outputs
        .iter()
        .find(|port| port.port == source_output_port)
        .ok_or_else(|| {
            finding(
                "dcall_source_port_invalid",
                format!("source artifact for service '{source_service_id}' declares no output port '{source_output_port}'"),
            )
        })?;

    let destination_artifact = dcode_registry_state
        .get(application_id, &destination_service_id)
        .ok_or_else(|| {
            finding(
                "dcall_destination_port_invalid",
                "destination D-Serv artifact is not registered",
            )
        })?;
    let matching_inputs: Vec<&crate::core::dserv_artifact::DServArtifactPortV1> =
        destination_artifact
        .ports
        .inputs
        .iter()
        .filter(|port| port.payload_schema == source_port_decl.payload_schema)
        .collect();
    let destination_port = match matching_inputs.as_slice() {
        [only] => only.port.clone(),
        [] => {
            return Err(finding(
                "dcall_destination_port_invalid",
                format!(
                    "destination artifact for service '{destination_service_id}' declares no input port with payload_schema '{}'",
                    source_port_decl.payload_schema
                ),
            ))
        }
        _ => {
            return Err(finding(
                "dcall_destination_port_invalid",
                format!(
                    "destination artifact for service '{destination_service_id}' declares MULTIPLE input ports with payload_schema '{}' -- v0.1 requires an unambiguous match",
                    source_port_decl.payload_schema
                ),
            ))
        }
    };

    let route_kind = if source_node_id == destination_node_id {
        ROUTE_KIND_LOCAL
    } else {
        ROUTE_KIND_REMOTE
    };

    Ok(DCallRouteV1 {
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        call_id: call_id.to_string(),
        dgraph_ref: dest_auth.dgraph_ref,
        dmap_ref: dest_auth.dmap_ref,
        source: DCallRouteEndpointV1 {
            service_id: source_service_id.to_string(),
            port: source_output_port.to_string(),
            node_id: source_node_id.to_string(),
            dserv_artifact_id: source_auth.dserv_artifact_id,
            dserv_artifact_digest: source_auth.dserv_artifact_digest,
            dserv_artifact_descriptor_digest: source_auth.dserv_artifact_descriptor_digest,
        },
        destination: DCallRouteEndpointV1 {
            service_id: destination_service_id,
            port: destination_port,
            node_id: destination_node_id,
            dserv_artifact_id: dest_auth.dserv_artifact_id,
            dserv_artifact_digest: dest_auth.dserv_artifact_digest,
            dserv_artifact_descriptor_digest: dest_auth.dserv_artifact_descriptor_digest,
        },
        route_kind: route_kind.to_string(),
        v2_dcall_realization,
    })
}

/// Deterministic order for fan-out (Phase 112 section 14): every DCall
/// edge in `dgraph.calls` whose `source_service_id` matches, sorted by
/// `call_id`. Zero matches is valid (not an error).
pub fn resolve_dcalls_for_emit<'a>(
    dgraph: &'a crate::core::canonical_dgraph::CanonicalDGraphV1,
    source_service_id: &str,
) -> Vec<&'a crate::core::canonical_dgraph::CanonicalDCallV1> {
    let mut matches: Vec<_> = dgraph
        .calls
        .iter()
        .filter(|call| call.source_service_id == source_service_id)
        .collect();
    matches.sort_by(|a, b| a.call_id.cmp(&b.call_id));
    matches
}

// --------------------------------------------------------------------
// Delivery grant
// --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Independent review fix v5 (section 9/11): an opaque, bare-hex
    /// SHA-256 digest reference to the exact governed source
    /// `DCodeInvocationEvidenceV1` that produced the emit this grant is for
    /// -- derived by DATUM (`dcode::evidence::dcode_evidence_sha256`) and
    /// sent as part of `RequestGrantRequest`. DServer does not possess the
    /// full source evidence record itself; it treats this purely as an
    /// opaque digest to persist and echo back, so a final DCall evidence
    /// record can forensically bind `source_dcode_evidence_sha256 ==
    /// grant.source_dcode_evidence_sha256 == recomputed digest(source
    /// evidence)` without ever duplicating the full record -- see
    /// `source-dcode-evidence-binding-audit.txt`. This is a forensic
    /// binding, not cryptographic remote attestation.
    pub source_dcode_evidence_sha256: String,
    pub route_kind: String,
    #[serde(default)]
    pub endpoint_id: Option<String>,
    #[serde(default)]
    pub endpoint_revision: Option<u64>,
    #[serde(default)]
    pub endpoint_descriptor_digest: Option<String>,
    /// Phase 113B2 (section 4/6/18): `Some(..)` exclusively for a DMap/2
    /// `via_diot_pubsub` grant; `None` for every DMap/1 grant. `#[serde(default)]`
    /// so a historical DMap/1 grant persisted before this field existed still
    /// deserializes (as `None`, never fabricated) -- see
    /// `validate_persisted_dcall_grant_state_integrity` for the reload-time
    /// shape/consistency checks this field is subject to.
    #[serde(default)]
    pub v2_dcall_transport_authority: Option<DCallRealizationTransportAuthorityV1>,
    pub issued_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DCallGrantStatus {
    Pending,
    Redeemed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DCallGrantRecord {
    pub grant: DCallDeliveryGrantV1,
    /// Bare-hex SHA-256 of the raw bearer token. The raw token is NEVER
    /// persisted, logged, or included in any evidence -- this hash is the
    /// only thing DServer ever stores about it (`dcall-grant-replay-audit.txt`).
    pub token_hash: String,
    pub status: DCallGrantStatus,
    #[serde(default)]
    pub redeemed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DCallGrantStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub grants: BTreeMap<String, DCallGrantRecord>,
}

impl DCallGrantStateV1 {
    pub fn empty() -> Self {
        Self {
            state_version: DCALL_GRANT_STATE_VERSION.to_string(),
            grants: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct IssuedGrant {
    pub grant: DCallDeliveryGrantV1,
    /// The raw high-entropy bearer token -- returned to the caller EXACTLY
    /// ONCE, here, at issuance. Never retrievable again afterward (not even
    /// by DServer itself -- only its hash is retained).
    pub token: String,
}

/// Phase 113B2 (section 12): the two shapes of delivery authority
/// `issue_grant` can accept, deliberately an ENUM rather than an
/// `Option<&DNodeTransportEndpointV1>` plus a separate optional DIoT
/// authority parameter -- a caller cannot construct both an HTTP endpoint
/// AND a DIoT transport authority at once, so "DMap/2 `via_diot_pubsub` +
/// HTTP endpoint authority as delivery authority" is unrepresentable, not
/// merely rejected at runtime.
pub enum DCallDeliveryAuthorityV1<'a> {
    /// DMap/1 local route: no delivery authority beyond the route itself.
    None,
    /// DMap/1 remote route: the declared HTTP transport endpoint for the
    /// destination node.
    Http(&'a DNodeTransportEndpointV1),
    /// DMap/2 `via_diot_pubsub` route (any `route_kind` -- section 11):
    /// the governed realization + runtime-binding authority, constructed by
    /// the caller (`post_request_grant`) from `derive_dcall_route`'s own
    /// `route.v2_dcall_realization` plus a resolved, current
    /// `diot_runtime_binding::DIoTRuntimeBindingV1`.
    DiotPubsub(DCallRealizationTransportAuthorityV1),
}

#[allow(clippy::too_many_arguments)]
pub fn issue_grant(
    grant_state: &mut DCallGrantStateV1,
    route: DCallRouteV1,
    delivery_authority: DCallDeliveryAuthorityV1,
    message_id: String,
    correlation_id: String,
    payload_sha256: String,
    payload_size: u64,
    source_dcode_evidence_sha256: String,
    now: DateTime<Utc>,
) -> Result<IssuedGrant, CanonicalValidationFinding> {
    // Section 12: reject the impossible authority combinations up front,
    // before any endpoint/token/state work. Because `DCallDeliveryAuthorityV1`
    // is an enum, "DMap/2 + HTTP endpoint as delivery authority" can never
    // reach this function at all (only `DiotPubsub` OR `Http`/`None`, never
    // both) -- the two combinations actually checked here are the ones the
    // type system cannot rule out by itself: DMap/1 route + DIoT authority,
    // and DMap/2 `via_diot_pubsub` route + missing DIoT authority.
    let v2_dcall_transport_authority = match (&route.v2_dcall_realization, &delivery_authority) {
        (Some(realization), DCallDeliveryAuthorityV1::DiotPubsub(authority)) => {
            // Defense in depth, not the primary authority-agreement proof:
            // `post_request_grant` is what actually resolves `authority`
            // from the CURRENT runtime-binding registry and cross-checks it
            // against `realization`; both are derived from the SAME loaded
            // `ddeploy_state` within the SAME lock hold (section 21/22), so
            // this equality is expected to always hold. It is checked again
            // here so `issue_grant` itself never persists a grant whose two
            // halves of authority silently disagree, even if a future call
            // site constructed `authority` some other way.
            if authority.realization_kind != realization.realization_kind
                || authority.diot_placement_id != realization.diot_placement_id
                || authority.interface != realization.interface
                || authority.channel != realization.channel
                || authority.dforward_id != realization.dforward_id
                || authority.dforward_revision != realization.dforward_revision
                || authority.dforward_digest != realization.dforward_digest
            {
                return Err(finding(
                    "dcall_diot_transport_authority_mismatch",
                    "supplied DIoT transport authority does not match the route's own realization authority",
                ));
            }
            Some(authority.clone())
        }
        (Some(realization), _) => {
            return Err(finding(
                "dcall_v2_realization_requires_diot_transport_authority",
                format!(
                    "D-Call '{}' is governed under an active DMap/2 realization ('{}') but no DIoT transport authority was supplied to issue_grant",
                    route.call_id, realization.realization_kind
                ),
            ));
        }
        (None, DCallDeliveryAuthorityV1::DiotPubsub(_)) => {
            return Err(finding(
                "dcall_v1_route_cannot_carry_diot_transport_authority",
                "D-Call route is governed under DMap/1 -- DIoT transport authority is only valid for an active DMap/2 via_diot_pubsub realization",
            ));
        }
        (None, _) => None,
    };

    // Section 10: HTTP endpoint authority is irrelevant for a DMap/2
    // `via_diot_pubsub` grant, regardless of `route_kind` -- a same-node OR
    // remote DIoT-realized call never requires (and never consults) an HTTP
    // transport endpoint. This block therefore only runs for the DMap/1 path.
    let http_endpoint = match &delivery_authority {
        DCallDeliveryAuthorityV1::Http(endpoint) => Some(*endpoint),
        _ => None,
    };
    if v2_dcall_transport_authority.is_none() && route.route_kind == ROUTE_KIND_REMOTE {
        let endpoint = http_endpoint.ok_or_else(|| {
            finding(
                "dcall_destination_endpoint_missing",
                format!("D-Call route to node '{}' is remote but no '{TRANSPORT_HTTP}' endpoint is declared for it", route.destination.node_id),
            )
        })?;
        if endpoint.node_id != route.destination.node_id {
            return Err(finding(
                "dcall_destination_endpoint_missing",
                "resolved endpoint does not belong to the destination node",
            ));
        }
    }

    let grant_id = format!("dcall-grant:{}", Uuid::new_v4());
    // Two concatenated UUIDv4s as the raw bearer token. Independent review
    // fix v3 (J7): an earlier version of this comment overstated this as
    // "256 bits of OS-CSPRNG-backed randomness" -- UUIDv4 fixes 6 of its 128
    // bits (the version and variant bits), so each UUIDv4 actually
    // contributes 122 random bits, and two concatenated contribute ~244, not
    // 256. That remains far more entropy than a v0.1 single-use, ~30-second-
    // lived grant token needs to resist brute-force guessing; the mechanism
    // itself is unchanged -- avoids adding a new dependency purely for
    // random-byte generation, reusing a mechanism already a direct
    // dependency of both crates.
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let token_hash = sha256_hex(token.as_bytes());
    let issued_at = now.to_rfc3339();
    let expires_at = (now + ChronoDuration::seconds(GRANT_TTL_SECONDS)).to_rfc3339();

    let grant = DCallDeliveryGrantV1 {
        grant_id: grant_id.clone(),
        project_id: route.project_id,
        application_id: route.application_id,
        call_id: route.call_id,
        dgraph_ref: route.dgraph_ref,
        dmap_ref: route.dmap_ref,
        source: route.source,
        destination: route.destination,
        message_id,
        correlation_id,
        payload_sha256,
        payload_size,
        source_dcode_evidence_sha256,
        route_kind: route.route_kind,
        endpoint_id: http_endpoint.map(|e| e.endpoint_id.clone()),
        endpoint_revision: http_endpoint.map(|e| e.revision),
        endpoint_descriptor_digest: http_endpoint.map(|e| e.descriptor_digest.clone()),
        v2_dcall_transport_authority,
        issued_at,
        expires_at,
    };

    grant_state.grants.insert(
        grant_id,
        DCallGrantRecord {
            grant: grant.clone(),
            token_hash,
            status: DCallGrantStatus::Pending,
            redeemed_at: None,
        },
    );

    Ok(IssuedGrant { grant, token })
}

#[derive(Debug)]
pub enum RedeemError {
    NotFound,
    TokenInvalid,
    Expired,
    AlreadyRedeemed,
    DestinationNodeMismatch,
    PayloadMismatch,
    StaleAuthority(CanonicalValidationFinding),
    /// Phase 113B3 (section 17): a DMap/2 `via_diot_pubsub` grant requires
    /// the caller to supply `observed_mqtt_topic` at redemption -- omitting
    /// it is a distinct failure from a WRONG topic (`ObservedTopicMismatch`)
    /// so an operator/log can tell "the destination listener never reported
    /// what it saw" from "it reported something and it was wrong".
    ObservedTopicMissing,
    /// Phase 113B3 (section 17): the caller-observed MQTT topic does not
    /// match the topic DServer independently recomputes from the grant's
    /// OWN bound `project_id`/`application_id`/`diot_placement_id`/
    /// `destination.node_id`/`channel`. This is a caller OBSERVATION to
    /// verify, never caller-supplied authority -- the expected value is
    /// never taken from the request. Leaves the grant `Pending` (this
    /// function returns before the `Redeemed` mutation).
    ObservedTopicMismatch,
}

/// Atomically redeems a grant: verifies token/expiry/destination/payload
/// identity, RE-CHECKS current canonical authority against the grant's
/// bound digests (a stale grant fails here even if it was valid at
/// issuance), and -- only if every check passes -- flips it to `Redeemed`
/// in the SAME state mutation (never a separate later "commit" step, so
/// there is no window where a caller could observe "validated but not yet
/// marked redeemed"). Returns the grant's bound routing data on success so
/// the caller can proceed to the actual D-Code invocation.
#[allow(clippy::too_many_arguments)]
pub fn redeem_grant(
    grant_state: &mut DCallGrantStateV1,
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    dcode_registry_state: &DCodeArtifactRegistryStateV1,
    dcall_endpoint_state: &DCallEndpointRegistryStateV1,
    diot_runtime_binding_state: &DIoTRuntimeBindingRegistryStateV1,
    grant_id: &str,
    token: &str,
    destination_node_id: &str,
    payload_sha256: &str,
    payload_size: u64,
    observed_mqtt_topic: Option<&str>,
    now: DateTime<Utc>,
) -> Result<DCallDeliveryGrantV1, RedeemError> {
    let record = grant_state
        .grants
        .get(grant_id)
        .ok_or(RedeemError::NotFound)?;

    if record.status == DCallGrantStatus::Redeemed {
        return Err(RedeemError::AlreadyRedeemed);
    }
    let token_hash = sha256_hex(token.as_bytes());
    if token_hash != record.token_hash {
        return Err(RedeemError::TokenInvalid);
    }
    let expires_at =
        DateTime::parse_from_rfc3339(&record.grant.expires_at).map_err(|_| RedeemError::Expired)?;
    if now > expires_at {
        return Err(RedeemError::Expired);
    }
    if record.grant.destination.node_id != destination_node_id {
        return Err(RedeemError::DestinationNodeMismatch);
    }
    if record.grant.payload_sha256 != payload_sha256 || record.grant.payload_size != payload_size {
        return Err(RedeemError::PayloadMismatch);
    }

    // Stale-authority recheck: re-derive BOTH the SOURCE and DESTINATION
    // authority chains right now and compare against what the grant bound
    // at issuance. Independent review v2 (I4): a prior version of this
    // check only re-verified the destination, which meant a source artifact
    // could change after grant issuance (limits/ports/safety/module/etc.)
    // while the destination stayed unchanged, and the grant would still
    // redeem -- the grant's own `source` field binds those exact identities,
    // so they must be re-checked with the same rigor as the destination.
    let current_source_auth = authorize_dcode_invocation(
        ddeploy_state,
        dnode_registry_state,
        dcode_registry_state,
        &record.grant.project_id,
        &record.grant.application_id,
        &record.grant.source.service_id,
        &record.grant.source.node_id,
    )
    .map_err(RedeemError::StaleAuthority)?;
    if current_source_auth.dmap_ref != record.grant.dmap_ref
        || current_source_auth.dgraph_ref != record.grant.dgraph_ref
        || current_source_auth.dserv_artifact_digest != record.grant.source.dserv_artifact_digest
        || current_source_auth.dserv_artifact_descriptor_digest
            != record.grant.source.dserv_artifact_descriptor_digest
    {
        return Err(RedeemError::StaleAuthority(finding(
            "dcall_stale_authority",
            format!("grant '{grant_id}' no longer matches current canonical SOURCE authority (dmap/dgraph/source artifact drifted since issuance)"),
        )));
    }

    let current_dest_auth = authorize_dcode_invocation(
        ddeploy_state,
        dnode_registry_state,
        dcode_registry_state,
        &record.grant.project_id,
        &record.grant.application_id,
        &record.grant.destination.service_id,
        &record.grant.destination.node_id,
    )
    .map_err(RedeemError::StaleAuthority)?;
    if current_dest_auth.dmap_ref != record.grant.dmap_ref
        || current_dest_auth.dgraph_ref != record.grant.dgraph_ref
        || current_dest_auth.dserv_artifact_digest != record.grant.destination.dserv_artifact_digest
        || current_dest_auth.dserv_artifact_descriptor_digest
            != record.grant.destination.dserv_artifact_descriptor_digest
    {
        return Err(RedeemError::StaleAuthority(finding(
            "dcall_stale_authority",
            format!("grant '{grant_id}' no longer matches current canonical DESTINATION authority (dmap/dgraph/artifact drifted since issuance)"),
        )));
    }
    // HTTP endpoint staleness is a DMap/1-only concern (section 10): a
    // DMap/2 `via_diot_pubsub` grant never carries endpoint authority
    // regardless of `route_kind`, so this check must not run for one (it
    // would otherwise incorrectly require/consult an HTTP endpoint that was
    // never this grant's delivery authority in the first place).
    if record.grant.v2_dcall_transport_authority.is_none() && record.grant.route_kind == ROUTE_KIND_REMOTE
    {
        let current_endpoint =
            dcall_endpoint_state.get(&record.grant.destination.node_id, TRANSPORT_HTTP);
        let current_digest = current_endpoint.map(|endpoint| endpoint.descriptor_digest.clone());
        if current_digest != record.grant.endpoint_descriptor_digest {
            return Err(RedeemError::StaleAuthority(finding(
                "dcall_stale_authority",
                format!(
                    "grant '{grant_id}' destination endpoint changed or was removed since issuance"
                ),
            )));
        }
    }

    // Phase 113B2 (section 14): a DMap/2 `via_diot_pubsub` grant must have
    // its FULL realization + runtime-binding authority re-proved current
    // before redemption -- the generic dmap/dgraph/artifact rechecks above
    // are necessary but not sufficient for this grant shape, since they say
    // nothing about the realization's channel/interface/placement/DForward
    // ref or about whether the DIoT runtime binding it names still exists
    // and is itself current. Any failure here leaves the grant `Pending`
    // (this function returns before the mutation below runs).
    if let Some(v2_authority) = &record.grant.v2_dcall_transport_authority {
        // "current D-Call route still exists" + "exact DGraph/DMap refs
        // agree" + "exact realization agrees": re-derive the route fresh,
        // from the grant's own bound source identity/port -- this is the
        // SAME production route-derivation function issuance itself uses,
        // never a second parallel interpretation of "current realization".
        let current_route = derive_dcall_route(
            ddeploy_state,
            dnode_registry_state,
            dcode_registry_state,
            &record.grant.project_id,
            &record.grant.application_id,
            &record.grant.call_id,
            &record.grant.source.service_id,
            &record.grant.source.node_id,
            &record.grant.source.port,
        )
        .map_err(RedeemError::StaleAuthority)?;

        let current_realization = current_route.v2_dcall_realization.as_ref().ok_or_else(|| {
            RedeemError::StaleAuthority(finding(
                "dcall_stale_authority",
                format!("grant '{grant_id}' was issued under a DMap/2 via_diot_pubsub realization but the currently active authority is no longer DMap/2"),
            ))
        })?;
        if current_realization.realization_kind != v2_authority.realization_kind
            || current_realization.diot_placement_id != v2_authority.diot_placement_id
            || current_realization.interface != v2_authority.interface
            || current_realization.channel != v2_authority.channel
            || current_realization.dforward_id != v2_authority.dforward_id
            || current_realization.dforward_revision != v2_authority.dforward_revision
            || current_realization.dforward_digest != v2_authority.dforward_digest
        {
            return Err(RedeemError::StaleAuthority(finding(
                "dcall_stale_authority",
                format!("grant '{grant_id}' no longer matches the currently active DMap/2 realization (kind/placement/interface/channel/DForward ref drifted since issuance)"),
            )));
        }

        // "exact runtime binding still exists" + "binding validates against
        // CURRENT authority": resolve by the SAME key issuance used, then
        // reuse the ONE production staleness validator -- never a duplicate
        // check.
        let current_binding = diot_runtime_binding_state
            .get(
                &record.grant.project_id,
                &record.grant.application_id,
                &v2_authority.diot_placement_id,
            )
            .ok_or_else(|| {
                RedeemError::StaleAuthority(finding(
                    "dcall_diot_runtime_binding_missing",
                    format!("grant '{grant_id}' names DIoT placement '{}' which no longer has a registered runtime binding", v2_authority.diot_placement_id),
                ))
            })?;
        verify_runtime_binding_still_current(current_binding, ddeploy_state, dnode_registry_state)
            .map_err(RedeemError::StaleAuthority)?;

        // "binding_id/binding_revision/descriptor_digest all match the
        // grant": the binding may still structurally validate yet no longer
        // be the SAME registration this grant was issued against (e.g. it
        // was re-registered at a new revision after issuance).
        if current_binding.binding_id != v2_authority.binding_id
            || current_binding.binding_revision != v2_authority.binding_revision
            || current_binding.descriptor_digest != v2_authority.descriptor_digest
        {
            return Err(RedeemError::StaleAuthority(finding(
                "dcall_stale_authority",
                format!("grant '{grant_id}' no longer matches the current DIoT runtime binding identity (binding_id/binding_revision/descriptor_digest drifted since issuance)"),
            )));
        }

        // Section 17: for a DMap/2 `via_diot_pubsub` grant, the destination
        // listener's OBSERVED physical MQTT topic is REQUIRED and must
        // exactly equal the topic DServer independently recomputes from the
        // grant's own bound identity -- this is a caller observation to
        // verify (proof the message actually arrived on the namespace this
        // exact realization/binding owns), never caller-supplied authority.
        // Historical DMap/1 HTTP redemption never supplies this field at all
        // (checked below, outside this `if let Some(v2_authority)` block, is
        // simply skipped for a DMap/1 grant).
        let observed = observed_mqtt_topic.ok_or(RedeemError::ObservedTopicMissing)?;
        let expected = compute_mqtt_request_topic(
            &record.grant.project_id,
            &record.grant.application_id,
            &v2_authority.diot_placement_id,
            &record.grant.destination.node_id,
            &v2_authority.channel,
        );
        if observed != expected {
            return Err(RedeemError::ObservedTopicMismatch);
        }
    }

    let grant = record.grant.clone();
    let entry = grant_state
        .grants
        .get_mut(grant_id)
        .expect("grant existence already confirmed above");
    entry.status = DCallGrantStatus::Redeemed;
    entry.redeemed_at = Some(now.to_rfc3339());
    Ok(grant)
}

/// Fail-closed relational integrity check, invoked from `load_state`.
/// Never repairs corruption; never allows a `Redeemed` grant to silently
/// revert (there is no code path here or in `redeem_grant` that ever
/// writes `Pending` over an existing `Redeemed` record).
pub fn validate_persisted_dcall_grant_state_integrity(
    state: &DCallGrantStateV1,
) -> Result<(), String> {
    if state.state_version != DCALL_GRANT_STATE_VERSION {
        return Err(format!(
            "unsupported dcall grant state_version '{}', expected '{DCALL_GRANT_STATE_VERSION}'",
            state.state_version
        ));
    }
    for (key, record) in &state.grants {
        if record.grant.grant_id.trim().is_empty() {
            return Err(format!(
                "dcall grant stored under key '{key}' has an empty grant_id"
            ));
        }
        if *key != record.grant.grant_id {
            return Err(format!(
                "dcall grant stored under key '{key}' does not match its own grant_id '{}'",
                record.grant.grant_id
            ));
        }
        if !is_valid_sha256_hex(&record.token_hash) {
            return Err(format!(
                "dcall grant '{key}' has a malformed token_hash (expected 64 lowercase hex chars)"
            ));
        }
        let issued_at = DateTime::parse_from_rfc3339(&record.grant.issued_at)
            .map_err(|_| format!("dcall grant '{key}' has an unparseable issued_at timestamp"))?;
        let expires_at = DateTime::parse_from_rfc3339(&record.grant.expires_at)
            .map_err(|_| format!("dcall grant '{key}' has an unparseable expires_at timestamp"))?;
        // I7: temporal relationships, not merely parseability.
        if expires_at <= issued_at {
            return Err(format!("dcall grant '{key}' has expires_at <= issued_at"));
        }
        match &record.status {
            DCallGrantStatus::Pending if record.redeemed_at.is_some() => {
                return Err(format!(
                    "dcall grant '{key}' is Pending but has a redeemed_at timestamp"
                ));
            }
            DCallGrantStatus::Pending => {}
            DCallGrantStatus::Redeemed => {
                let redeemed_at_str = record.redeemed_at.as_ref().ok_or_else(|| {
                    format!("dcall grant '{key}' is Redeemed but has no redeemed_at timestamp")
                })?;
                let redeemed_at = DateTime::parse_from_rfc3339(redeemed_at_str).map_err(|_| {
                    format!("dcall grant '{key}' has an unparseable redeemed_at timestamp")
                })?;
                if redeemed_at < issued_at || redeemed_at > expires_at {
                    return Err(format!(
                        "dcall grant '{key}' has redeemed_at outside [issued_at, expires_at]"
                    ));
                }
            }
        }
        // I7: route_kind must be one of the two known values, and (section
        // 15) its node-identity implication always holds regardless of
        // delivery-authority shape -- local means source/destination are the
        // SAME D-Node, remote means they are DIFFERENT ones.
        match record.grant.route_kind.as_str() {
            ROUTE_KIND_LOCAL => {
                if record.grant.source.node_id != record.grant.destination.node_id {
                    return Err(format!(
                        "dcall grant '{key}' has route_kind=local but source.node_id '{}' != destination.node_id '{}'",
                        record.grant.source.node_id, record.grant.destination.node_id
                    ));
                }
            }
            ROUTE_KIND_REMOTE => {
                if record.grant.source.node_id == record.grant.destination.node_id {
                    return Err(format!("dcall grant '{key}' has route_kind=remote but source.node_id == destination.node_id '{}'", record.grant.source.node_id));
                }
            }
            other => {
                return Err(format!(
                    "dcall grant '{key}' has unsupported route_kind '{other}'"
                ))
            }
        }
        // Section 13: delivery-authority shape -- exactly one of "HTTP
        // endpoint authority" (DMap/1) or "DIoT transport authority"
        // (DMap/2 `via_diot_pubsub`) may be present, never both, and each
        // present shape must itself be well-formed. Deliberately does NOT
        // require the DIoT authority to agree with CURRENT topology here --
        // a persisted grant remains a historically valid RECORD even after
        // topology later changes; current usability is rechecked separately
        // at redemption (`redeem_grant`'s own dedicated block), never here.
        match &record.grant.v2_dcall_transport_authority {
            Some(v2) => {
                if record.grant.endpoint_id.is_some()
                    || record.grant.endpoint_revision.is_some()
                    || record.grant.endpoint_descriptor_digest.is_some()
                {
                    return Err(format!(
                        "dcall grant '{key}' carries BOTH v2_dcall_transport_authority and HTTP endpoint authority -- these are mutually exclusive"
                    ));
                }
                if v2.realization_kind != "via_diot_pubsub" {
                    return Err(format!(
                        "dcall grant '{key}' has v2_dcall_transport_authority.realization_kind '{}' -- v0.1 only ever issues 'via_diot_pubsub'",
                        v2.realization_kind
                    ));
                }
                for (label, value) in [
                    ("diot_placement_id", &v2.diot_placement_id),
                    ("interface", &v2.interface),
                    ("channel", &v2.channel),
                    ("dforward_id", &v2.dforward_id),
                    ("binding_id", &v2.binding_id),
                ] {
                    if value.trim().is_empty() {
                        return Err(format!(
                            "dcall grant '{key}' has an empty v2_dcall_transport_authority.{label}"
                        ));
                    }
                }
                // Mirrors `CanonicalDMapV2::validate`'s own wildcard-channel
                // rejection (`canonical_dmap_v2.rs`) -- no shared function
                // exists to call, so this is the identical inline check, not
                // a reimplementation of different logic.
                if v2.channel.contains('+') || v2.channel.contains('#') {
                    return Err(format!(
                        "dcall grant '{key}' has a wildcard v2_dcall_transport_authority.channel (forbidden)"
                    ));
                }
                if v2.dforward_revision < 1 {
                    return Err(format!(
                        "dcall grant '{key}' has v2_dcall_transport_authority.dforward_revision {} < 1",
                        v2.dforward_revision
                    ));
                }
                if !is_canonical_digest_format(&v2.dforward_digest) {
                    return Err(format!("dcall grant '{key}' has a malformed v2_dcall_transport_authority.dforward_digest (expected the canonical 'sha256:<64 lowercase hex>' form)"));
                }
                if v2.binding_revision < 1 {
                    return Err(format!(
                        "dcall grant '{key}' has v2_dcall_transport_authority.binding_revision {} < 1",
                        v2.binding_revision
                    ));
                }
                if !is_canonical_digest_format(&v2.descriptor_digest) {
                    return Err(format!("dcall grant '{key}' has a malformed v2_dcall_transport_authority.descriptor_digest (expected the canonical 'sha256:<64 lowercase hex>' form)"));
                }
                if v2.transport != crate::core::diot_runtime_binding::TRANSPORT_MQTT {
                    return Err(format!(
                        "dcall grant '{key}' has unsupported v2_dcall_transport_authority.transport '{}'",
                        v2.transport
                    ));
                }
            }
            None => match record.grant.route_kind.as_str() {
                ROUTE_KIND_LOCAL => {
                    if record.grant.endpoint_id.is_some()
                        || record.grant.endpoint_revision.is_some()
                        || record.grant.endpoint_descriptor_digest.is_some()
                    {
                        return Err(format!(
                            "dcall grant '{key}' has route_kind=local but carries endpoint fields"
                        ));
                    }
                }
                ROUTE_KIND_REMOTE => {
                    let endpoint_id = record.grant.endpoint_id.as_deref().ok_or_else(|| {
                        format!("dcall grant '{key}' has route_kind=remote but is missing endpoint_id")
                    })?;
                    let endpoint_revision = record.grant.endpoint_revision.ok_or_else(|| format!("dcall grant '{key}' has route_kind=remote but is missing endpoint_revision"))?;
                    let endpoint_descriptor_digest =
                        record.grant.endpoint_descriptor_digest.as_deref().ok_or_else(|| format!("dcall grant '{key}' has route_kind=remote but is missing endpoint_descriptor_digest"))?;
                    // J4: presence alone is not enough -- shape/value validity too.
                    if endpoint_id.trim().is_empty() {
                        return Err(format!("dcall grant '{key}' has an empty endpoint_id"));
                    }
                    if endpoint_revision < 1 {
                        return Err(format!(
                            "dcall grant '{key}' has endpoint_revision {endpoint_revision} < 1"
                        ));
                    }
                    if !is_canonical_digest_format(endpoint_descriptor_digest) {
                        return Err(format!("dcall grant '{key}' has a malformed endpoint_descriptor_digest (expected the canonical 'sha256:<64 lowercase hex>' form)"));
                    }
                }
                // Already rejected above -- unreachable here.
                _ => {}
            },
        }
        // J4: `dgraph_ref`/`dmap_ref` must themselves be structurally valid
        // canonical contract references, not merely present.
        for (label, contract_ref) in [
            ("dgraph_ref", &record.grant.dgraph_ref),
            ("dmap_ref", &record.grant.dmap_ref),
        ] {
            if contract_ref.id.trim().is_empty() {
                return Err(format!("dcall grant '{key}' has an empty {label}.id"));
            }
            if contract_ref.revision < 1 {
                return Err(format!(
                    "dcall grant '{key}' has {label}.revision {} < 1",
                    contract_ref.revision
                ));
            }
            if !is_canonical_digest_format(&contract_ref.digest) {
                return Err(format!("dcall grant '{key}' has a malformed {label}.digest (expected the canonical 'sha256:<64 lowercase hex>' form)"));
            }
        }
        // J4: source/destination endpoint identity -- non-empty port/artifact
        // id, shape-valid module digest (bare hex) and descriptor digest
        // (canonical prefixed form).
        for (label, endpoint) in [
            ("source", &record.grant.source),
            ("destination", &record.grant.destination),
        ] {
            if endpoint.port.trim().is_empty() {
                return Err(format!("dcall grant '{key}' has an empty {label}.port"));
            }
            if endpoint.dserv_artifact_id.trim().is_empty() {
                return Err(format!(
                    "dcall grant '{key}' has an empty {label}.dserv_artifact_id"
                ));
            }
            if !is_valid_sha256_hex(&endpoint.dserv_artifact_digest) {
                return Err(format!("dcall grant '{key}' has a malformed {label}.dserv_artifact_digest (expected 64 lowercase hex chars)"));
            }
            if !is_canonical_digest_format(&endpoint.dserv_artifact_descriptor_digest) {
                return Err(format!("dcall grant '{key}' has a malformed {label}.dserv_artifact_descriptor_digest (expected the canonical 'sha256:<64 lowercase hex>' form)"));
            }
        }
        // I7: payload digest shape and non-empty required identifiers.
        if !is_valid_sha256_hex(&record.grant.payload_sha256) {
            return Err(format!("dcall grant '{key}' has a malformed payload_sha256 (expected 64 lowercase hex chars)"));
        }
        // Independent review fix v5 (section 15): the source D-Code evidence
        // digest reference is opaque to DServer, but its SHAPE must still be
        // a real SHA-256 hex string -- never silently repaired.
        if !is_valid_sha256_hex(&record.grant.source_dcode_evidence_sha256) {
            return Err(format!("dcall grant '{key}' has a malformed source_dcode_evidence_sha256 (expected 64 lowercase hex chars)"));
        }
        for (label, value) in [
            ("project_id", &record.grant.project_id),
            ("application_id", &record.grant.application_id),
            ("call_id", &record.grant.call_id),
            ("message_id", &record.grant.message_id),
            ("correlation_id", &record.grant.correlation_id),
            ("source.service_id", &record.grant.source.service_id),
            ("source.node_id", &record.grant.source.node_id),
            (
                "destination.service_id",
                &record.grant.destination.service_id,
            ),
            ("destination.node_id", &record.grant.destination.node_id),
        ] {
            if value.trim().is_empty() {
                return Err(format!(
                    "dcall grant '{key}' has an empty required identifier '{label}'"
                ));
            }
        }
    }
    Ok(())
}

pub fn load_state(db_path: &std::path::Path) -> Result<DCallGrantStateV1, String> {
    match load_domain_json::<DCallGrantStateV1>(db_path, DOMAIN_DCALL_GRANT_STATE)? {
        Some(state) => {
            validate_persisted_dcall_grant_state_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DCallGrantStateV1::empty()),
    }
}

pub fn persist_state(db_path: &std::path::Path, state: &DCallGrantStateV1) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DCALL_GRANT_STATE, state)
}

#[cfg(test)]
mod tests;
