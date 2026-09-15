//! Active-D-Serv -> D-Code invocation authorization (ADR-0021, 111.9/111.10).
//!
//! A D-Code invocation must never be authorized merely because a canonical
//! D-Serv artifact happens to exist. `authorize_dcode_invocation` proves the
//! full chain from CURRENT accepted canonical authority (D-Deploy's active
//! D-Map, the D-Graph it was accepted against, and the D-Node structural
//! registry) down to one specific artifact digest, exactly as ADR-0021
//! requires. `verify_dcode_authorization_still_current` re-derives that
//! same chain and fails closed (`stale_dcode_authorization`) the moment ANY
//! of it has moved -- the service was reassigned, the D-Node's declared
//! capabilities changed, or the registered artifact changed -- following
//! the same "recompute and compare, never trust a cached snapshot" pattern
//! Stage 111's `AcceptError::StaleAuthority` (`ddeploy.rs`) already
//! established for D-Continuum authority.
//!
//! This module never mutates `DDeployStateV1`, `DNodeRegistryStateV1`, or
//! `DCodeArtifactRegistryStateV1` -- it only reads already-loaded state
//! snapshots, so the caller (the API layer) is responsible for acquiring
//! them under the established `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION`
//! stable-view lock order before calling in, exactly like
//! `post_accept_proposal`/`post_plan_proposal` already do.

use crate::core::canonical_dgraph::DNODE_ABI_V0;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalValidationFinding};
use crate::core::ddeploy::{application_key, DDeployStateV1};
use crate::core::dcode_registry::DCodeArtifactRegistryStateV1;
use crate::core::dnode_registry::DNodeRegistryStateV1;
use crate::core::dserv_artifact::recompute_dserv_artifact_digest;

pub const F_DCODE_APPLICATION_NOT_ACTIVE: &str = "dcode_application_not_active";
pub const F_DCODE_SERVICE_NOT_IN_DGRAPH: &str = "dcode_service_not_in_dgraph";
pub const F_DCODE_SERVICE_NOT_PLACED_ON_NODE: &str = "dcode_service_not_placed_on_node";
pub const F_DCODE_EXECUTION_REQUIREMENT_UNSUPPORTED: &str = "dcode_execution_requirement_unsupported";
pub const F_DCODE_NODE_UNKNOWN: &str = "dcode_node_unknown";
pub const F_DCODE_NODE_ABI_UNSUPPORTED: &str = "dcode_node_abi_unsupported";
pub const F_DCODE_ARTIFACT_NOT_FOUND: &str = "dcode_artifact_not_found";
pub const F_DCODE_ARTIFACT_ABI_MISMATCH: &str = "dcode_artifact_abi_mismatch";
pub const F_DCODE_ARTIFACT_INVALID: &str = "dcode_artifact_invalid";
pub const F_DCODE_STALE_AUTHORIZATION: &str = "stale_dcode_authorization";
/// Independent review v3 (F1): `constraints.allowed_stages` is a real,
/// structurally-valid field on `datum.dserv-artifact/1` (non-empty values
/// appear in the checked-in fixtures), but canonical D-Node structural
/// authority (`dnode_registry`) carries NO authoritative stage identity --
/// only node_id/platform/capacity/ABI. Rather than inventing one from
/// Sentinel lease state, free-text stage, live telemetry, or
/// ServiceArtifact `target_stages` (none of which are canonical D-Node
/// authority), v0.1 fails closed: a non-empty `allowed_stages` cannot
/// currently be verified at all, so authorization is refused rather than
/// silently ignoring a constraint the artifact explicitly declares. This
/// does NOT make the descriptor itself invalid -- `validate()` still
/// accepts it -- it only means it is not yet EXECUTABLE under v0.1.
pub const F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE: &str = "dcode_artifact_stage_constraint_unverifiable";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

/// Immutable binding data proving one specific D-Code invocation is
/// currently authorized. Carries enough of each referenced contract's own
/// identity (id + revision + digest, or the artifact's own content digest)
/// that `verify_dcode_authorization_still_current` can detect ANY drift by
/// recomputation and comparison -- never by trusting this struct's fields
/// as self-certifying.
///
/// Independent review v2 (R2A) correction: `dserv_artifact_digest` and
/// `dserv_artifact_descriptor_digest` are two DELIBERATELY SEPARATE
/// identities, never one field doing double duty. `dserv_artifact_digest`
/// is the D-Code MODULE's own content digest (`artifact.dcode.sha256`) --
/// it changes only when the executable bytes change. `dserv_artifact_descriptor_digest`
/// is the canonical JCS digest of the ENTIRE artifact descriptor
/// (`recompute_dserv_artifact_digest`) -- it changes if ANYTHING in the
/// descriptor changes, including `limits`, `ports`, `safety`, or `abi`,
/// even while the module bytes (and therefore `dserv_artifact_digest`)
/// stay identical. Before this field existed, a caller could keep the
/// module SHA-256 constant while silently raising `fuel_per_invocation`,
/// widening `ports`, or flipping a `safety` flag, and neither
/// `verify_dcode_authorization_still_current` nor the real DATUM-side
/// `invoke_dcode` execution path would notice -- both checked only the
/// module digest and ABI. Binding the full descriptor digest closes that
/// gap at both the server verify path and the real execution path (see
/// `DATUM::dcode::invoke::invoke_dcode`, which now independently recomputes
/// and checks this same field against the artifact it actually loads).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DCodeInvocationAuthorizationV1 {
    pub authorization_id: String,
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
    pub node_id: String,
    pub dmap_ref: CanonicalContractReferenceV1,
    pub dgraph_ref: CanonicalContractReferenceV1,
    pub dserv_artifact_id: String,
    /// D-Code MODULE content digest only (`artifact.dcode.sha256`) -- see
    /// the struct doc comment above for why this is deliberately distinct
    /// from `dserv_artifact_descriptor_digest`.
    pub dserv_artifact_digest: String,
    /// Canonical JCS digest of the WHOLE `datum.dserv-artifact/1` descriptor
    /// (`recompute_dserv_artifact_digest`) -- see the struct doc comment.
    pub dserv_artifact_descriptor_digest: String,
    pub dnode_abi: String,
}

pub fn authorize_dcode_invocation(
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    dcode_registry_state: &DCodeArtifactRegistryStateV1,
    project_id: &str,
    application_id: &str,
    service_id: &str,
    node_id: &str,
) -> Result<DCodeInvocationAuthorizationV1, CanonicalValidationFinding> {
    // 1. application has an ACTIVE canonical accepted DMap.
    let active = ddeploy_state
        .active_by_application
        .get(&application_key(project_id, application_id))
        .filter(|active| active.active_dmap.project_id == project_id)
        .ok_or_else(|| {
            finding(
                F_DCODE_APPLICATION_NOT_ACTIVE,
                format!("project '{project_id}' application '{application_id}' has no active accepted D-Map"),
            )
        })?;

    // 2. service_id exists in that application's immutable DGraph snapshot,
    //    and its execution_requirement is the v0.1 D-Node ABI.
    let proposal = ddeploy_state.proposals.get(&active.proposal_id).ok_or_else(|| {
        finding(
            F_DCODE_APPLICATION_NOT_ACTIVE,
            format!("active application '{application_id}' references proposal '{}', which is missing", active.proposal_id),
        )
    })?;
    let service = proposal
        .dgraph_snapshot
        .services
        .iter()
        .find(|service| service.service_id == service_id)
        .ok_or_else(|| {
            finding(
                F_DCODE_SERVICE_NOT_IN_DGRAPH,
                format!("service '{service_id}' does not exist in application '{application_id}''s active D-Graph"),
            )
        })?;
    if service.execution_requirement.dnode_abi != DNODE_ABI_V0 {
        return Err(finding(
            F_DCODE_EXECUTION_REQUIREMENT_UNSUPPORTED,
            format!(
                "service '{service_id}' requires D-Node ABI '{}', not the D-Code-supported '{DNODE_ABI_V0}'",
                service.execution_requirement.dnode_abi
            ),
        ));
    }

    // 3. active DMap places that service on the requested D-Node.
    let placement = active
        .active_dmap
        .placements
        .iter()
        .find(|placement| placement.service_id == service_id)
        .ok_or_else(|| {
            finding(
                F_DCODE_SERVICE_NOT_PLACED_ON_NODE,
                format!("service '{service_id}' has no placement in the active D-Map"),
            )
        })?;
    if placement.node_id != node_id {
        return Err(finding(
            F_DCODE_SERVICE_NOT_PLACED_ON_NODE,
            format!(
                "service '{service_id}' is placed on node '{}', not the requested node '{node_id}'",
                placement.node_id
            ),
        ));
    }

    // 5. current authoritative D-Node declares support for datum-dnode/0.
    let node = dnode_registry_state.nodes.get(node_id).ok_or_else(|| {
        finding(F_DCODE_NODE_UNKNOWN, format!("node '{node_id}' is not a currently declared structural D-Node"))
    })?;
    if !node.capabilities.dnode_abi_versions.iter().any(|abi| abi == DNODE_ABI_V0) {
        return Err(finding(
            F_DCODE_NODE_ABI_UNSUPPORTED,
            format!("node '{node_id}' does not declare support for '{DNODE_ABI_V0}'"),
        ));
    }

    // 6/7/8. registered canonical D-Serv artifact belongs to this
    // service/application, its ABI matches, and its content digest is
    // canonically valid.
    let artifact = dcode_registry_state.get(application_id, service_id).ok_or_else(|| {
        finding(
            F_DCODE_ARTIFACT_NOT_FOUND,
            format!("no canonical D-Serv artifact is registered for application '{application_id}' service '{service_id}'"),
        )
    })?;
    let artifact_findings = artifact.validate();
    if !artifact_findings.is_empty() {
        return Err(finding(
            F_DCODE_ARTIFACT_INVALID,
            format!("registered D-Serv artifact for service '{service_id}' no longer validates cleanly: {artifact_findings:?}"),
        ));
    }
    let artifact_abi = format!("{}/{}", artifact.abi.name, artifact.abi.version);
    if artifact_abi != DNODE_ABI_V0 {
        return Err(finding(
            F_DCODE_ARTIFACT_ABI_MISMATCH,
            format!("registered D-Serv artifact declares ABI '{artifact_abi}', expected '{DNODE_ABI_V0}'"),
        ));
    }

    // F1: a non-empty stage constraint cannot currently be verified against
    // any canonical authority -- fail closed rather than silently ignore it.
    if !artifact.constraints.allowed_stages.is_empty() {
        return Err(finding(
            F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE,
            format!(
                "registered D-Serv artifact for service '{service_id}' declares constraints.allowed_stages={:?}, but canonical D-Node structural authority carries no authoritative stage identity to verify this against in v0.1 -- failing closed rather than silently ignoring the constraint",
                artifact.constraints.allowed_stages
            ),
        ));
    }

    let dgraph_digest = recompute_dgraph_digest(&proposal.dgraph_snapshot)
        .map_err(|error| finding(F_DCODE_ARTIFACT_INVALID, format!("recompute D-Graph digest: {error}")))?;
    let descriptor_digest = recompute_dserv_artifact_digest(artifact)
        .map_err(|error| finding(F_DCODE_ARTIFACT_INVALID, format!("recompute D-Serv artifact descriptor digest: {error}")))?;

    Ok(DCodeInvocationAuthorizationV1 {
        authorization_id: format!(
            "dcode-auth:{project_id}:{application_id}:{service_id}:{node_id}:{}",
            active.active_dmap.revision
        ),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        service_id: service_id.to_string(),
        node_id: node_id.to_string(),
        dmap_ref: CanonicalContractReferenceV1 {
            id: active.active_dmap.dmap_id.clone(),
            revision: active.active_dmap.revision,
            digest: active.dmap_digest.clone(),
        },
        dgraph_ref: CanonicalContractReferenceV1 {
            id: proposal.dgraph_snapshot.dgraph_id.clone(),
            revision: proposal.dgraph_snapshot.revision,
            digest: dgraph_digest,
        },
        dserv_artifact_id: artifact.artifact_id.clone(),
        dserv_artifact_digest: artifact.dcode.sha256.clone(),
        dserv_artifact_descriptor_digest: descriptor_digest,
        dnode_abi: DNODE_ABI_V0.to_string(),
    })
}

/// Re-derives the SAME authorization chain right now and fails closed if
/// any immutable-binding field has drifted since `authorization` was
/// issued. Never trusts `authorization`'s own fields as proof of anything
/// -- only as the baseline to compare a fresh computation against.
pub fn verify_dcode_authorization_still_current(
    authorization: &DCodeInvocationAuthorizationV1,
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    dcode_registry_state: &DCodeArtifactRegistryStateV1,
) -> Result<(), CanonicalValidationFinding> {
    let current = authorize_dcode_invocation(
        ddeploy_state,
        dnode_registry_state,
        dcode_registry_state,
        &authorization.project_id,
        &authorization.application_id,
        &authorization.service_id,
        &authorization.node_id,
    )?;

    if current.dmap_ref != authorization.dmap_ref
        || current.dgraph_ref != authorization.dgraph_ref
        || current.dserv_artifact_digest != authorization.dserv_artifact_digest
        || current.dserv_artifact_descriptor_digest != authorization.dserv_artifact_descriptor_digest
        || current.dnode_abi != authorization.dnode_abi
    {
        return Err(finding(
            F_DCODE_STALE_AUTHORIZATION,
            format!(
                "authorization '{}' no longer matches current canonical authority (dmap/dgraph/artifact/artifact-descriptor/abi drifted)",
                authorization.authorization_id
            ),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1, CanonicalDNodeV1,
    };
    use crate::core::canonical_dgraph::{
        CanonicalDGraphV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1,
        CanonicalDServV1,
    };
    use crate::core::canonical_dmap::{CanonicalDMapV1, CanonicalDServPlacementV1};
    use crate::core::ddeploy::{
        application_key, canonical_dmap_id, ActiveApplicationDeployment, ProposalRecord,
    };
    use crate::core::operational_dgraph::OperationalDesiredService;
    use crate::core::dcode_registry::{declare_dserv_artifact, DCodeArtifactRegistryStateV1};
    use crate::core::dserv_artifact::{
        CanonicalDServArtifactV1, DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1,
        DServArtifactLimitsV1, DServArtifactPortsV1, DServArtifactSafetyV1, CANONICAL_DSERV_ARTIFACT_SCHEMA,
        DNODE_ABI_V0_REQUIRED_EXPORTS,
    };
    use crate::core::dnode_registry::{declare_dnode, DNodeRegistryStateV1};
    use serde_json::Value;
    use std::collections::BTreeMap;

    const PROJECT: &str = "proj:lab";
    const APP: &str = "app:sensor-pipeline";
    const SVC: &str = "decode";
    const NODE: &str = "fog-01";

    fn dgraph() -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: crate::core::canonical_dgraph::CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: APP.to_string(),
            dgraph_id: "dgraph:sensor-pipeline".to_string(),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_id: SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
            }],
            calls: Vec::new(),
        }
    }

    fn node() -> CanonicalDNodeV1 {
        CanonicalDNodeV1 {
            node_id: NODE.to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "x86_64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_000_000_000 },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        }
    }

    fn artifact() -> CanonicalDServArtifactV1 {
        CanonicalDServArtifactV1 {
            schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: "dserv:sensor-pipeline:decode".to_string(),
            version: "0.1.0".to_string(),
            application_id: APP.to_string(),
            dserv_id: SVC.to_string(),
            dcode: DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{}", "a".repeat(64)),
                sha256: "a".repeat(64),
                content_id: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 64,
            },
            abi: DServArtifactAbiV1 {
                name: "datum-dnode".to_string(),
                version: "0".to_string(),
                required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
                allowed_imports: Vec::new(),
            },
            ports: DServArtifactPortsV1::default(),
            limits: DServArtifactLimitsV1 {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 100,
                max_memory_pages: 4,
                max_message_bytes: 4096,
                max_emits_per_invocation: 1,
            },
            safety: DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: DServArtifactConstraintsV1::default(),
            qos: Value::Null,
            provenance: Value::Null,
            probes: Value::Null,
            signature_ref: None,
        }
    }

    fn setup() -> (DDeployStateV1, DNodeRegistryStateV1, DCodeArtifactRegistryStateV1) {
        let proposal_id = "prop-1".to_string();
        let mut ddeploy_state = DDeployStateV1::empty();
        ddeploy_state.proposals.insert(
            proposal_id.clone(),
            ProposalRecord {
                proposal: crate::core::ddeploy::DDeployProposalV1 {
                    schema: crate::core::ddeploy::DDEPLOY_PROPOSAL_SCHEMA.to_string(),
                    proposal_id: proposal_id.clone(),
                    project_id: PROJECT.to_string(),
                    application_id: APP.to_string(),
                    source: crate::core::ddeploy::ProposalSource::Deterministic,
                    created_at_utc: "2026-01-01T00:00:00Z".to_string(),
                    expected_active_dmap_revision: None,
                    dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                        id: dgraph().dgraph_id,
                        revision: dgraph().revision,
                        digest: recompute_dgraph_digest(&dgraph()).unwrap(),
                    },
                    dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                        id: "dcontinuum:lab".to_string(),
                        revision: 1,
                        digest: format!("sha256:{}", "1".repeat(64)),
                    },
                    placements: vec![CanonicalDServPlacementV1 { service_id: SVC.to_string(), node_id: NODE.to_string() }],
                },
                planner_provenance: None,
                proposal_digest: String::new(),
                dgraph_snapshot: dgraph(),
                dcontinuum_snapshot: crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
                    schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
                    project_id: PROJECT.to_string(),
                    dcontinuum_id: "dcontinuum:lab".to_string(),
                    revision: 1,
                    nodes: vec![node()],
                    links: Vec::new(),
                },
                state: crate::core::ddeploy::ProposalLifecycleState::Accepted,
                accepted_acceptance_id: Some("acc-1".to_string()),
            },
        );

        let dmap = CanonicalDMapV1 {
            schema: crate::core::canonical_dmap::CANONICAL_DMAP_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            dmap_id: canonical_dmap_id(PROJECT, APP),
            revision: 1,
            dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                id: dgraph().dgraph_id,
                revision: dgraph().revision,
                digest: recompute_dgraph_digest(&dgraph()).unwrap(),
            },
            dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                id: "dcontinuum:lab".to_string(),
                revision: 1,
                digest: format!("sha256:{}", "1".repeat(64)),
            },
            placements: vec![CanonicalDServPlacementV1 { service_id: SVC.to_string(), node_id: NODE.to_string() }],
        };
        let dmap_digest = crate::core::canonical_dmap::recompute_dmap_digest(&dmap).unwrap();

        let mut operational_services = BTreeMap::new();
        operational_services.insert(
            SVC.to_string(),
            OperationalDesiredService {
                service_id: SVC.to_string(),
                runtime: "wasm".to_string(),
                desired_state: "running".to_string(),
            },
        );

        ddeploy_state.active_by_application.insert(
            application_key(PROJECT, APP),
            ActiveApplicationDeployment {
                application_id: APP.to_string(),
                active_dmap: dmap,
                dmap_digest,
                proposal_id,
                acceptance_id: "acc-1".to_string(),
                accepted_at_utc: "2026-01-01T00:00:01Z".to_string(),
                operational_services,
            },
        );

        let mut dnode_registry_state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode_registry_state, node()).unwrap();

        let mut dcode_registry_state = DCodeArtifactRegistryStateV1::empty();
        declare_dserv_artifact(&mut dcode_registry_state, artifact()).unwrap();

        (ddeploy_state, dnode_registry_state, dcode_registry_state)
    }

    #[test]
    fn correctly_placed_service_authorization_succeeds() {
        let (ddeploy_state, dnode_registry_state, dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .expect("authorization should succeed");
        assert_eq!(authorization.node_id, NODE);
        assert_eq!(authorization.dserv_artifact_digest, "a".repeat(64));
    }

    #[test]
    fn service_not_active_is_rejected() {
        let (ddeploy_state, dnode_registry_state, dcode_registry_state) = setup();
        let error =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, "app:unknown", SVC, NODE)
                .unwrap_err();
        assert_eq!(error.code, F_DCODE_APPLICATION_NOT_ACTIVE);
    }

    #[test]
    fn service_active_on_different_node_is_rejected() {
        let (ddeploy_state, dnode_registry_state, dcode_registry_state) = setup();
        let error =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, "cloud-01")
                .unwrap_err();
        assert_eq!(error.code, F_DCODE_SERVICE_NOT_PLACED_ON_NODE);
    }

    #[test]
    fn artifact_belonging_to_wrong_service_is_rejected() {
        let (ddeploy_state, dnode_registry_state, mut dcode_registry_state) = setup();
        dcode_registry_state.artifacts.clear();
        let mut wrong = artifact();
        wrong.dserv_id = "other-service".to_string();
        declare_dserv_artifact(&mut dcode_registry_state, wrong).unwrap();
        let error =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap_err();
        assert_eq!(error.code, F_DCODE_ARTIFACT_NOT_FOUND);
    }

    #[test]
    fn dnode_lacking_abi_is_rejected() {
        let (ddeploy_state, mut dnode_registry_state, dcode_registry_state) = setup();
        dnode_registry_state.nodes.get_mut(NODE).unwrap().capabilities.dnode_abi_versions.clear();
        let error =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap_err();
        assert_eq!(error.code, F_DCODE_NODE_ABI_UNSUPPORTED);
    }

    #[test]
    fn active_dmap_changing_after_authorization_is_detected_as_stale() {
        let (mut ddeploy_state, dnode_registry_state, dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap();

        // Move the service to a different node by mutating the active D-Map
        // in place (simulating a later acceptance that changed placement).
        let active = ddeploy_state.active_by_application.get_mut(&application_key(PROJECT, APP)).unwrap();
        active.active_dmap.placements[0].node_id = "cloud-02".to_string();
        active.dmap_digest = crate::core::canonical_dmap::recompute_dmap_digest(&active.active_dmap).unwrap();

        let error = verify_dcode_authorization_still_current(
            &authorization,
            &ddeploy_state,
            &dnode_registry_state,
            &dcode_registry_state,
        )
        .unwrap_err();
        assert_eq!(error.code, F_DCODE_SERVICE_NOT_PLACED_ON_NODE);
    }

    #[test]
    fn artifact_changing_after_authorization_is_detected_as_stale() {
        let (ddeploy_state, dnode_registry_state, mut dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap();

        // Replace the registered artifact with a different-content one
        // under a new artifact_id (declare_dserv_artifact fails closed on
        // an in-place conflicting overwrite, exactly as it should).
        dcode_registry_state.artifacts.clear();
        let mut changed = artifact();
        changed.artifact_id = "dserv:sensor-pipeline:decode:v2".to_string();
        changed.dcode.sha256 = "c".repeat(64);
        changed.dcode.content_id = format!("sha256:{}", "c".repeat(64));
        changed.dcode.uri = format!("datum-blob://sha256/{}", "c".repeat(64)); // keep F7A's uri/sha256/content_id binding consistent
        declare_dserv_artifact(&mut dcode_registry_state, changed).unwrap();

        let error = verify_dcode_authorization_still_current(
            &authorization,
            &ddeploy_state,
            &dnode_registry_state,
            &dcode_registry_state,
        )
        .unwrap_err();
        assert_eq!(error.code, F_DCODE_STALE_AUTHORIZATION);
    }

    /// R2 regression: changing ONLY a descriptor field that has nothing to
    /// do with module content (here, `limits.fuel_per_invocation`) while the
    /// module SHA-256 stays byte-identical must still be caught as stale --
    /// proving `dserv_artifact_descriptor_digest` is genuinely load-bearing,
    /// not merely a duplicate of the module digest.
    #[test]
    fn changing_only_limits_invalidates_the_descriptor_digest_binding() {
        let (ddeploy_state, dnode_registry_state, mut dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap();

        dcode_registry_state.artifacts.clear();
        let mut changed = artifact();
        changed.limits.fuel_per_invocation *= 2; // module bytes/sha256 UNCHANGED
        declare_dserv_artifact(&mut dcode_registry_state, changed).unwrap();

        let error = verify_dcode_authorization_still_current(
            &authorization,
            &ddeploy_state,
            &dnode_registry_state,
            &dcode_registry_state,
        )
        .unwrap_err();
        assert_eq!(error.code, F_DCODE_STALE_AUTHORIZATION);
    }

    /// Same proof for `ports` -- widening the declared output ports must
    /// also be caught even though the module digest never changes.
    #[test]
    fn changing_only_ports_invalidates_the_descriptor_digest_binding() {
        let (ddeploy_state, dnode_registry_state, mut dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap();

        dcode_registry_state.artifacts.clear();
        let mut changed = artifact();
        changed.ports.outputs.push(crate::core::dserv_artifact::DServArtifactPortV1 {
            port: "extra".to_string(),
            payload_schema: "datum://schemas/extra/1".to_string(),
            external: false,
        });
        declare_dserv_artifact(&mut dcode_registry_state, changed).unwrap();

        let error = verify_dcode_authorization_still_current(
            &authorization,
            &ddeploy_state,
            &dnode_registry_state,
            &dcode_registry_state,
        )
        .unwrap_err();
        assert_eq!(error.code, F_DCODE_STALE_AUTHORIZATION);
    }

    /// Same proof for a field with no independent check of its own -- `qos`
    /// -- widening/changing it must also be caught even though the module
    /// digest never changes.
    ///
    /// Independent review v2 (R7) note: this test originally flipped a
    /// `safety` flag (e.g. `network_access = true`) to prove the same point,
    /// but R7 now makes EVERY `safety` flag's only valid value `false` for
    /// zero-import v0.1 D-Code -- `declare_dserv_artifact` itself rejects any
    /// artifact with a `true` safety flag before this test could ever reach
    /// the descriptor-digest comparison being proven here.
    ///
    /// Independent review v3 (F1) note: a second replacement field was then
    /// needed -- this test was rewritten to use `constraints.allowed_stages`
    /// instead, but F1 now makes ANY non-empty `allowed_stages` fail closed
    /// at `authorize_dcode_invocation` itself (`dcode_artifact_stage_constraint_unverifiable`),
    /// which `verify_dcode_authorization_still_current` calls internally --
    /// so that change is now caught by F1's own check before the descriptor
    /// digest comparison this test aims to isolate is ever reached. `qos` is
    /// used instead: an opaque `serde_json::Value` field that is part of the
    /// canonical descriptor digest but has no independent semantic check of
    /// its own anywhere in this module, so changing it can ONLY be caught by
    /// the descriptor-digest binding this test proves is load-bearing.
    #[test]
    fn changing_only_qos_invalidates_the_descriptor_digest_binding() {
        let (ddeploy_state, dnode_registry_state, mut dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap();

        dcode_registry_state.artifacts.clear();
        let mut changed = artifact();
        changed.qos = serde_json::json!({ "max_handler_latency_ms": 999 });
        declare_dserv_artifact(&mut dcode_registry_state, changed).unwrap();

        let error = verify_dcode_authorization_still_current(
            &authorization,
            &ddeploy_state,
            &dnode_registry_state,
            &dcode_registry_state,
        )
        .unwrap_err();
        assert_eq!(error.code, F_DCODE_STALE_AUTHORIZATION);
    }

    /// F1 regression 1: empty `allowed_stages` (the default in `artifact()`)
    /// imposes no restriction and authorization succeeds -- already
    /// implicitly proven by `correctly_placed_service_authorization_succeeds`
    /// above, restated explicitly here for clarity.
    #[test]
    fn empty_allowed_stages_can_authorize() {
        let (ddeploy_state, dnode_registry_state, dcode_registry_state) = setup();
        assert!(artifact().constraints.allowed_stages.is_empty());
        let result =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE);
        assert!(result.is_ok(), "empty allowed_stages must not restrict authorization");
    }

    /// F1 regressions 2+3: a non-empty `allowed_stages` fails closed as
    /// currently unverifiable, and does so BEFORE any
    /// `DCodeInvocationAuthorizationV1` is ever returned (the function
    /// returns `Err`, never a partially-valid `Ok`).
    #[test]
    fn nonempty_allowed_stages_fails_closed_before_any_authorization_is_returned() {
        let (ddeploy_state, dnode_registry_state, mut dcode_registry_state) = setup();
        dcode_registry_state.artifacts.clear();
        let mut restricted = artifact();
        restricted.constraints.allowed_stages.push("S3-Fog".to_string());
        declare_dserv_artifact(&mut dcode_registry_state, restricted).unwrap();

        let result =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE);
        let error = result.expect_err("non-empty allowed_stages must fail closed, never return an authorization");
        assert_eq!(error.code, F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE);
    }

    /// F1 regression 4: the stage-constraint check is a pure function of the
    /// artifact descriptor -- it never consults Sentinel registration/lease
    /// state or any live/free-text stage identity. A cheap structural guard
    /// (mirroring the pattern `dcode::worker`'s own source-guard test uses)
    /// keeps this true even if this file grows.
    #[test]
    fn stage_constraint_check_never_consults_sentinel_or_live_state() {
        // Scan only the NON-TEST, NON-DOC-COMMENT lines of this file --
        // otherwise this guard trivially "finds" the forbidden strings
        // inside its own assertion literals below (the same self-detection
        // pitfall the ddeploy.rs source guard already had to work around),
        // or inside a doc comment that legitimately explains WHY those
        // sources are deliberately NOT consulted (see the
        // `F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE` doc comment
        // above, which names them for exactly that reason).
        let source = include_str!("dcode_authorization.rs");
        let production_code: String = source
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("///") && !trimmed.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            production_code.len() < source.len(),
            "the #[cfg(test)] split boundary must actually be found, or this guard checks nothing"
        );
        let lower = production_code.to_lowercase();
        assert!(!lower.contains("sentinel_registration"), "must never consult Sentinel registration state");
        assert!(!lower.contains("target_stages"), "must never consult ServiceArtifact target_stages");
        assert!(!lower.contains("resource_observation"), "must never consult live telemetry/resource observation");
    }

    /// F1 regression 5: the descriptor digest persisted in the authorization
    /// still binds `constraints.allowed_stages` -- changing it (while it
    /// stays empty->empty is a no-op, so this proves the NON-empty case is
    /// itself digest-bound too, independent of the F1 authorization-time
    /// rejection) changes the descriptor digest. This is a narrower, purely
    /// digest-level check (no authorization call), since a live
    /// authorize/verify round-trip with a non-empty value now always fails
    /// closed via F1 before reaching digest comparison.
    #[test]
    fn descriptor_digest_still_binds_allowed_stages() {
        let base = artifact();
        let mut changed = artifact();
        changed.constraints.allowed_stages.push("S3-Fog".to_string());
        assert_ne!(
            crate::core::dserv_artifact::recompute_dserv_artifact_digest(&base).unwrap(),
            crate::core::dserv_artifact::recompute_dserv_artifact_digest(&changed).unwrap(),
            "constraints.allowed_stages must be part of the canonical descriptor digest"
        );
    }

    #[test]
    fn unchanged_authority_remains_current() {
        let (ddeploy_state, dnode_registry_state, dcode_registry_state) = setup();
        let authorization =
            authorize_dcode_invocation(&ddeploy_state, &dnode_registry_state, &dcode_registry_state, PROJECT, APP, SVC, NODE)
                .unwrap();
        assert!(verify_dcode_authorization_still_current(
            &authorization,
            &ddeploy_state,
            &dnode_registry_state,
            &dcode_registry_state,
        )
        .is_ok());
    }
}
