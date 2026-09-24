use super::*;
use crate::core::canonical_dcontinuum::{
    CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1,
};
use crate::core::canonical_dgraph::{
    recompute_dgraph_digest, CanonicalDCallV1, CanonicalDGraphV1,
    CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1, CanonicalDServV1,
    CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
};
use crate::core::canonical_dmap::{
    recompute_dmap_digest, CanonicalDMapV1, CanonicalDServPlacementV1, CANONICAL_DMAP_SCHEMA,
};
use crate::core::dcode_registry::{declare_dserv_artifact, DCodeArtifactRegistryStateV1};
use crate::core::ddeploy::{
    application_key, canonical_dmap_id, ActiveApplicationDeployment, DDeployProposalV1,
    ProposalLifecycleState, ProposalRecord, ProposalSource, DDEPLOY_PROPOSAL_SCHEMA,
};
use crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1;
use crate::core::dnode_registry::{declare_dnode, DNodeRegistryStateV1};
use crate::core::dserv_artifact::{
    CanonicalDServArtifactV1, DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1,
    DServArtifactLimitsV1, DServArtifactPortV1, DServArtifactPortsV1, DServArtifactSafetyV1,
    CANONICAL_DSERV_ARTIFACT_SCHEMA, DNODE_ABI_V0_REQUIRED_EXPORTS,
};
use crate::core::operational_dgraph::OperationalDesiredService;
use serde_json::Value;
use std::collections::BTreeMap;

const PROJECT: &str = "proj:lab";
const APP: &str = "app:pipeline";
const SOURCE_SVC: &str = "sensor";
const DEST_SVC: &str = "decoder";
const CALL_ID: &str = "sensor-to-decoder";
const OUTPUT_PORT: &str = "raw";
const INPUT_PORT: &str = "in";
const PAYLOAD_SCHEMA: &str = "raw-bytes/1";

fn dgraph() -> CanonicalDGraphV1 {
    CanonicalDGraphV1 {
        schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
        application_id: APP.to_string(),
        dgraph_id: "dgraph:pipeline".to_string(),
        revision: 1,
        services: vec![
            CanonicalDServV1 {
                service_id: SOURCE_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 1,
                },
            },
            CanonicalDServV1 {
                service_id: DEST_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 1,
                },
            },
        ],
        calls: vec![CanonicalDCallV1 {
            call_id: CALL_ID.to_string(),
            source_service_id: SOURCE_SVC.to_string(),
            target_service_id: DEST_SVC.to_string(),
        }],
    }
}

fn node(node_id: &str) -> CanonicalDNodeV1 {
    CanonicalDNodeV1 {
        node_id: node_id.to_string(),
        platform: CanonicalNodePlatformV1 {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
        },
        capacity: CanonicalNodeCapacityV1 {
            cpu_cores: 4,
            memory_bytes: 4_000_000_000,
        },
        capabilities: CanonicalNodeCapabilitiesV1 {
            dnode_abi_versions: vec![DNODE_ABI_V0.to_string()],
        },
    }
}

fn artifact(service_id: &str, ports: DServArtifactPortsV1) -> CanonicalDServArtifactV1 {
    CanonicalDServArtifactV1 {
        schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
        artifact_id: format!("dserv:pipeline:{service_id}"),
        version: "0.1.0".to_string(),
        application_id: APP.to_string(),
        dserv_id: service_id.to_string(),
        dcode: DServArtifactDCodeV1 {
            uri: format!("datum-blob://sha256/{}", "a".repeat(64)),
            sha256: "a".repeat(64),
            content_id: format!("sha256:{}", "a".repeat(64)),
            size_bytes: 64,
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
        ports,
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

fn source_artifact() -> CanonicalDServArtifactV1 {
    artifact(
        SOURCE_SVC,
        DServArtifactPortsV1 {
            inputs: Vec::new(),
            outputs: vec![DServArtifactPortV1 {
                port: OUTPUT_PORT.to_string(),
                payload_schema: PAYLOAD_SCHEMA.to_string(),
                external: false,
            }],
        },
    )
}

fn dest_artifact() -> CanonicalDServArtifactV1 {
    artifact(
        DEST_SVC,
        DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: INPUT_PORT.to_string(),
                payload_schema: PAYLOAD_SCHEMA.to_string(),
                external: false,
            }],
            outputs: Vec::new(),
        },
    )
}

/// `source_node`/`dest_node` may be the SAME id (-> local route) or
/// different ids (-> remote route, and BOTH nodes must be structurally
/// declared).
struct Fixture {
    ddeploy: DDeployStateV1,
    dnode: DNodeRegistryStateV1,
    dcode: DCodeArtifactRegistryStateV1,
}

fn build_fixture(source_node: &str, dest_node: &str) -> Fixture {
    let proposal_id = "prop-1".to_string();
    let mut ddeploy = DDeployStateV1::empty();
    let dg = dgraph();
    let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();

    ddeploy.proposals.insert(
        proposal_id.clone(),
        ProposalRecord {
            proposal: DDeployProposalV1 {
                schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
                proposal_id: proposal_id.clone(),
                project_id: PROJECT.to_string(),
                application_id: APP.to_string(),
                source: ProposalSource::Deterministic,
                created_at_utc: "2026-01-01T00:00:00Z".to_string(),
                expected_active_dmap_revision: None,
                dgraph_ref: CanonicalContractReferenceV1 {
                    id: dg.dgraph_id.clone(),
                    revision: dg.revision,
                    digest: dgraph_digest.clone(),
                },
                dcontinuum_ref: CanonicalContractReferenceV1 {
                    id: "dcontinuum:lab".to_string(),
                    revision: 1,
                    digest: format!("sha256:{}", "1".repeat(64)),
                },
                placements: vec![
                    CanonicalDServPlacementV1 {
                        service_id: SOURCE_SVC.to_string(),
                        node_id: source_node.to_string(),
                    },
                    CanonicalDServPlacementV1 {
                        service_id: DEST_SVC.to_string(),
                        node_id: dest_node.to_string(),
                    },
                ],
            },
            planner_provenance: None,
            proposal_digest: String::new(),
            dgraph_snapshot: dg.clone(),
            dcontinuum_snapshot: crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
                schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
                project_id: PROJECT.to_string(),
                dcontinuum_id: "dcontinuum:lab".to_string(),
                revision: 1,
                nodes: vec![node(source_node), node(dest_node)],
                links: Vec::new(),
            },
            state: ProposalLifecycleState::Accepted,
            accepted_acceptance_id: Some("acc-1".to_string()),
        },
    );

    let dmap = CanonicalDMapV1 {
        schema: CANONICAL_DMAP_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        application_id: APP.to_string(),
        dmap_id: canonical_dmap_id(PROJECT, APP),
        revision: 1,
        dgraph_ref: CanonicalContractReferenceV1 {
            id: dg.dgraph_id.clone(),
            revision: dg.revision,
            digest: dgraph_digest,
        },
        dcontinuum_ref: CanonicalContractReferenceV1 {
            id: "dcontinuum:lab".to_string(),
            revision: 1,
            digest: format!("sha256:{}", "1".repeat(64)),
        },
        placements: vec![
            CanonicalDServPlacementV1 {
                service_id: SOURCE_SVC.to_string(),
                node_id: source_node.to_string(),
            },
            CanonicalDServPlacementV1 {
                service_id: DEST_SVC.to_string(),
                node_id: dest_node.to_string(),
            },
        ],
    };
    let dmap_digest = recompute_dmap_digest(&dmap).unwrap();

    let mut operational_services = BTreeMap::new();
    for svc in [SOURCE_SVC, DEST_SVC] {
        operational_services.insert(
            svc.to_string(),
            OperationalDesiredService {
                service_id: svc.to_string(),
                runtime: "wasm".to_string(),
                desired_state: "running".to_string(),
            },
        );
    }

    ddeploy.active_by_application.insert(
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

    let mut dnode = DNodeRegistryStateV1::empty();
    declare_dnode(&mut dnode, node(source_node)).unwrap();
    if dest_node != source_node {
        declare_dnode(&mut dnode, node(dest_node)).unwrap();
    }

    let mut dcode = DCodeArtifactRegistryStateV1::empty();
    declare_dserv_artifact(&mut dcode, source_artifact()).unwrap();
    declare_dserv_artifact(&mut dcode, dest_artifact()).unwrap();

    Fixture {
        ddeploy,
        dnode,
        dcode,
    }
}

fn derive(
    fixture: &Fixture,
    source_node: &str,
) -> Result<DCallRouteV1, CanonicalValidationFinding> {
    derive_dcall_route(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        CALL_ID,
        SOURCE_SVC,
        source_node,
        OUTPUT_PORT,
    )
}

// ---------------------------------------------------------------- ROUTE

#[test]
fn valid_call_edge_produces_a_local_route_on_same_node() {
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    assert_eq!(route.route_kind, ROUTE_KIND_LOCAL);
    assert_eq!(route.destination.service_id, DEST_SVC);
    assert_eq!(route.destination.node_id, "fog-01");
    assert_eq!(route.destination.port, INPUT_PORT);
}

#[test]
fn valid_call_edge_produces_a_remote_route_across_nodes() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    assert_eq!(route.route_kind, ROUTE_KIND_REMOTE);
    assert_eq!(route.destination.node_id, "cloud-01");
}

#[test]
fn caller_cannot_override_destination_node_or_service() {
    // `derive_dcall_route`'s parameter list has no destination-node or
    // destination-service parameter at all -- the only way to prove this
    // at the API boundary is a source-guard test on the HTTP request type,
    // which lives in `api::dcall`'s own tests. Here we prove the DERIVED
    // destination is independent of anything the source-side caller
    // supplies: two callers claiming to be different source nodes for the
    // SAME call still resolve to the SAME destination.
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    assert_eq!(route.destination.service_id, DEST_SVC);
    assert_eq!(route.destination.node_id, "cloud-01");
}

#[test]
fn unknown_call_id_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let error = derive_dcall_route(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        "no-such-call",
        SOURCE_SVC,
        "fog-01",
        OUTPUT_PORT,
    )
            .unwrap_err();
    assert_eq!(error.code, "dcall_not_found");
}

#[test]
fn call_id_with_wrong_source_service_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let error = derive_dcall_route(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        CALL_ID,
        DEST_SVC,
        "fog-01",
        OUTPUT_PORT,
    )
    .unwrap_err();
    assert_eq!(error.code, "dcall_not_found");
}

#[test]
fn source_port_not_declared_as_output_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let error = derive_dcall_route(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        CALL_ID,
        SOURCE_SVC,
        "fog-01",
        "not-a-real-port",
    )
        .unwrap_err();
    assert_eq!(error.code, "dcall_source_port_invalid");
}

#[test]
fn destination_with_no_matching_input_port_is_rejected() {
    let mut fixture = build_fixture("fog-01", "fog-01");
    // Redeclare the destination artifact with a DIFFERENT payload_schema so
    // no input port matches the source's output port anymore. The registry
    // is immutable-per-key, so build a fresh state with the mismatched
    // artifact from scratch instead of trying to mutate in place.
    let mut dcode = DCodeArtifactRegistryStateV1::empty();
    declare_dserv_artifact(&mut dcode, source_artifact()).unwrap();
    let mismatched = artifact(
        DEST_SVC,
        DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: INPUT_PORT.to_string(),
                payload_schema: "different-schema/1".to_string(),
                external: false,
            }],
            outputs: Vec::new(),
        },
    );
    declare_dserv_artifact(&mut dcode, mismatched).unwrap();
    fixture.dcode = dcode;
    let error = derive(&fixture, "fog-01").unwrap_err();
    assert_eq!(error.code, "dcall_destination_port_invalid");
}

#[test]
fn destination_with_ambiguous_matching_input_ports_is_rejected() {
    let mut fixture = build_fixture("fog-01", "fog-01");
    let mut dcode = DCodeArtifactRegistryStateV1::empty();
    declare_dserv_artifact(&mut dcode, source_artifact()).unwrap();
    let ambiguous = artifact(
        DEST_SVC,
        DServArtifactPortsV1 {
            inputs: vec![
                DServArtifactPortV1 {
                    port: "in-a".to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                },
                DServArtifactPortV1 {
                    port: "in-b".to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                },
            ],
            outputs: Vec::new(),
        },
    );
    declare_dserv_artifact(&mut dcode, ambiguous).unwrap();
    fixture.dcode = dcode;
    let error = derive(&fixture, "fog-01").unwrap_err();
    assert_eq!(error.code, "dcall_destination_port_invalid");
}

#[test]
fn destination_service_with_no_active_placement_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    // Remove the destination's placement from the active DMap while
    // leaving the DGraph call edge intact.
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.active_by_application.get_mut(&key).unwrap();
    active
        .active_dmap
        .placements
        .retain(|p| p.service_id != DEST_SVC);
    let error = derive_dcall_route(
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        CALL_ID,
        SOURCE_SVC,
        "fog-01",
        OUTPUT_PORT,
    )
    .unwrap_err();
    assert_eq!(error.code, "dcall_destination_service_not_placed");
}

/// Phase 113B4A regression: `GET /api/v1/dcall/calls/:project/:app/:service`
/// (`get_dcalls_for_source_service`) previously read ONLY the DMap/1-
/// specific `active_by_application` map directly instead of the shared
/// `active_placement_authority` accessor every other Phase-112 consumer
/// (`derive_dcall_route`, `authorize_dcode_invocation`) already migrated to
/// in Phase 113A -- so an application whose ONLY active authority is
/// DMap/2 was wrongly reported as having no active D-Map at all. This was
/// undetected until the real governed MQTT E2E test (113B4A) became the
/// first exerciser of D-Call dispatch discovery against a DMap/2-only
/// application. This proves the handler's own fixed logic (consult
/// `active_placement_authority`, then `resolve_dcalls_for_emit` against its
/// `dgraph_snapshot`) against a purely DMap/2-active fixture, with no
/// DMap/1 authority present at all.
#[test]
fn dcall_discovery_finds_calls_for_a_dmap2_only_active_application() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    assert!(
        fixture.ddeploy.active_by_application.is_empty(),
        "this fixture must be DMap/2-only -- no v1 active authority at all"
    );
    let active = crate::core::ddeploy::active_placement_authority(&fixture.ddeploy, PROJECT, APP)
        .unwrap()
        .expect("a DMap/2-only application must still report an active placement authority");
    assert!(active.v2.is_some(), "this fixture's active authority must be DMap/2");
    let dgraph_snapshot = active
        .dgraph_snapshot
        .expect("active placement authority must carry its DGraph snapshot");
    let matches = resolve_dcalls_for_emit(&dgraph_snapshot, SOURCE_SVC);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].call_id, CALL_ID);
}

#[test]
fn resolve_dcalls_for_emit_is_deterministic_and_empty_is_valid() {
    let dg = dgraph();
    let matches = resolve_dcalls_for_emit(&dg, SOURCE_SVC);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].call_id, CALL_ID);
    let none = resolve_dcalls_for_emit(&dg, DEST_SVC);
    assert!(none.is_empty());
}

// ---------------------------------------------------------------- GRANTS

fn now() -> DateTime<Utc> {
    Utc::now()
}

#[test]
fn issuing_a_local_grant_needs_no_endpoint() {
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    assert_eq!(issued.grant.route_kind, ROUTE_KIND_LOCAL);
    assert!(!issued.token.is_empty());
}

#[test]
fn issuing_a_remote_grant_without_an_endpoint_fails_closed() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let error = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap_err();
    assert_eq!(error.code, "dcall_destination_endpoint_missing");
}

fn sample_endpoint(node_id: &str, revision: u64) -> DNodeTransportEndpointV1 {
    let mut endpoint = DNodeTransportEndpointV1 {
        node_id: node_id.to_string(),
        transport: TRANSPORT_HTTP.to_string(),
        endpoint_id: format!("dcall-endpoint:{node_id}:http"),
        revision,
        base_url: "http://127.0.0.1:9100".to_string(),
        descriptor_digest: String::new(),
    };
    endpoint.descriptor_digest =
        crate::core::dcall_endpoint::recompute_endpoint_digest(&endpoint).unwrap();
    endpoint
}

#[test]
fn raw_token_is_never_persisted_only_its_hash() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let endpoint = sample_endpoint("cloud-01", 1);
    let mut grants = DCallGrantStateV1::empty();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::Http(&endpoint),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get(&issued.grant.grant_id).unwrap();
    assert_ne!(record.token_hash, issued.token);
    assert_eq!(record.token_hash.len(), 64);
    let serialized = serde_json::to_string(record).unwrap();
    assert!(!serialized.contains(&issued.token));
}

fn issue_local(fixture: &Fixture) -> (DCallGrantStateV1, IssuedGrant) {
    let route = derive(fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    (grants, issued)
}

#[test]
fn first_redemption_succeeds_second_fails_with_already_redeemed() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    redeem_grant(&mut grants, &fixture.ddeploy, &fixture.dnode, &fixture.dcode, &endpoints, &diot_bindings, &issued.grant.grant_id, &issued.token, "fog-01", "deadbeef", 4, None, now())
        .expect("first redemption should succeed");
    let error = redeem_grant(&mut grants, &fixture.ddeploy, &fixture.dnode, &fixture.dcode, &endpoints, &diot_bindings, &issued.grant.grant_id, &issued.token, "fog-01", "deadbeef", 4, None, now())
        .unwrap_err();
    assert!(matches!(error, RedeemError::AlreadyRedeemed));
}

#[test]
fn wrong_token_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        "wrong-token",
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::TokenInvalid));
}

#[test]
fn expired_grant_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let far_future = now() + ChronoDuration::seconds(GRANT_TTL_SECONDS + 60);
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None,
        far_future,
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::Expired));
}

#[test]
fn wrong_destination_node_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "cloud-01",
        "deadbeef",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::DestinationNodeMismatch));
}

#[test]
fn payload_digest_mismatch_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "not-the-real-digest",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::PayloadMismatch));
}

#[test]
fn payload_size_mismatch_is_rejected() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        999,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::PayloadMismatch));
}

#[test]
fn stale_dmap_after_grant_issuance_fails_redemption() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    // Simulate the active DMap having moved on (revision bumped) after the
    // grant was issued -- redemption must recompute a DIFFERENT dmap_ref
    // and fail closed rather than trust the grant's stale binding.
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.active_by_application.get_mut(&key).unwrap();
    active.active_dmap.revision = 2;
    active.dmap_digest = "sha256:".to_string() + &"9".repeat(64);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(f) if f.code == "dcall_stale_authority"));
}

#[test]
fn moved_destination_after_grant_issuance_fails_redemption() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.active_by_application.get_mut(&key).unwrap();
    for placement in active.active_dmap.placements.iter_mut() {
        if placement.service_id == DEST_SVC {
            placement.node_id = "cloud-99".to_string();
        }
    }
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    // The grant still names the OLD destination node ("fog-01"), so
    // redemption is attempted against that node -- but the active DMap no
    // longer places the destination service there at all, so
    // `authorize_dcode_invocation` itself now fails for that (service,
    // node) pair.
    let error = redeem_grant(
        &mut grants,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
}

/// I4: redemption re-authorizes BOTH sides -- a SOURCE artifact descriptor
/// change (with the module digest unchanged) after grant issuance must fail
/// redemption exactly as a destination change already did.
#[test]
fn source_artifact_descriptor_change_after_grant_issuance_fails_redemption() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    // Simulate post-issuance drift by directly replacing the registry entry
    // (the dcode_registry's own immutable-conflict-only `declare_dserv_artifact`
    // gate is Phase-111 machinery already tested elsewhere; this test isolates
    // the redemption-time RECHECK, not the declaration-time immutability rule).
    let mut dcode = fixture.dcode.clone();
    let key = crate::core::dcode_registry::dserv_artifact_key(APP, SOURCE_SVC);
    let mut changed_source = source_artifact();
    changed_source.limits.timeout_ms += 1; // descriptor digest changes, module digest unchanged
    dcode.artifacts.insert(key, changed_source);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(&mut grants, &fixture.ddeploy, &fixture.dnode, &dcode, &endpoints, &diot_bindings, &issued.grant.grant_id, &issued.token, "fog-01", "deadbeef", 4, None, now())
        .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)), "expected StaleAuthority, got {error:?}");
}

/// I4: a source SERVICE moved to a different node after grant issuance must
/// also fail redemption (the grant's bound `source.node_id` no longer
/// matches current active-DMap placement).
#[test]
fn source_service_moved_after_grant_issuance_fails_redemption() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.active_by_application.get_mut(&key).unwrap();
    for placement in active.active_dmap.placements.iter_mut() {
        if placement.service_id == SOURCE_SVC {
            placement.node_id = "cloud-99".to_string();
        }
    }
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
}

/// I4: an UNCHANGED grant (both source and destination authority identical
/// to issuance time) must still redeem normally -- the two-sided recheck
/// must not be a false-positive source of failure for the common case.
#[test]
fn unchanged_grant_redeems_normally_after_two_sided_recheck() {
    let fixture = build_fixture("fog-01", "fog-01");
    let (mut grants, issued) = issue_local(&fixture);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
        .expect("an unchanged grant must redeem successfully");
}

#[test]
fn changed_endpoint_revision_after_grant_issuance_fails_remote_redemption() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let endpoint_v1 = sample_endpoint("cloud-01", 1);
    let mut grants = DCallGrantStateV1::empty();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::Http(&endpoint_v1),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();

    let mut endpoints = DCallEndpointRegistryStateV1::empty();

    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let endpoint_v2 = sample_endpoint("cloud-01", 2);
    endpoints.endpoints.insert(
        crate::core::dcall_endpoint::endpoint_key("cloud-01", TRANSPORT_HTTP),
        endpoint_v2,
    );

    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "cloud-01",
        "deadbeef",
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(f) if f.code == "dcall_stale_authority"));
}

// -------------------------------------------------- K2 / SECTION 13: STALE
// SOURCE EXECUTION BINDING (cheap unit-level variants; the section-13
// Real HTTP issuance variants and deployment-change races are covered in
// DATUM/tests/dcall_two_node_integration.rs, including SQLite insertion checks.

fn matching_binding(route: &DCallRouteV1) -> SourceExecutionBindingV1 {
    SourceExecutionBindingV1 {
        service_id: route.source.service_id.clone(),
        node_id: route.source.node_id.clone(),
        dgraph_id: route.dgraph_ref.id.clone(),
        dgraph_revision: route.dgraph_ref.revision,
        dgraph_digest: route.dgraph_ref.digest.clone(),
        dmap_id: route.dmap_ref.id.clone(),
        dmap_revision: route.dmap_ref.revision,
        dmap_digest: route.dmap_ref.digest.clone(),
        dserv_artifact_id: route.source.dserv_artifact_id.clone(),
        dserv_artifact_digest: route.source.dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: route.source.dserv_artifact_descriptor_digest.clone(),
    }
}

/// Phase 113B2: registers a real DIoT runtime binding for `route`'s own
/// realization (against `fixture`'s CURRENT, unmutated authority) and builds
/// the exact `DCallDeliveryAuthorityV1::DiotPubsub` a legitimate
/// `issue_grant` call for this route requires. Used by the (small number of)
/// pre-113B2 tests that issue a grant for a `build_v2_fixture` route purely
/// to exercise generic redemption/staleness mechanics that are NOT
/// themselves about the DIoT transport authority -- `issue_grant` now
/// unconditionally requires this authority for any DMap/2 `via_diot_pubsub`
/// route, so these tests must supply one to reach the mechanic under test.
fn diot_transport_authority_for(
    fixture: &Fixture,
    route: &DCallRouteV1,
    diot_bindings: &mut DIoTRuntimeBindingRegistryStateV1,
) -> DCallRealizationTransportAuthorityV1 {
    let realization = route
        .v2_dcall_realization
        .as_ref()
        .expect("route must be governed under an active DMap/2 realization");
    let binding = crate::core::diot_runtime_binding::register_runtime_binding(
        diot_bindings,
        &fixture.ddeploy,
        &fixture.dnode,
        PROJECT,
        APP,
        &realization.diot_placement_id,
        "mqtt://broker.lab:1883",
        "2026-01-01T00:00:00Z",
    )
    .expect("registering a runtime binding against the fixture's own active DMap/2 must succeed");
    DCallRealizationTransportAuthorityV1 {
        realization_kind: realization.realization_kind.clone(),
        diot_placement_id: realization.diot_placement_id.clone(),
        interface: realization.interface.clone(),
        channel: realization.channel.clone(),
        dforward_id: realization.dforward_id.clone(),
        dforward_revision: realization.dforward_revision,
        dforward_digest: realization.dforward_digest.clone(),
        binding_id: binding.binding_id,
        binding_revision: binding.binding_revision,
        descriptor_digest: binding.descriptor_digest,
        transport: binding.transport,
    }
}

#[test]
fn matching_source_execution_binding_is_accepted() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let binding = matching_binding(&route);
    assert!(check_source_execution_binding(&route, &binding).is_ok());
}

#[test]
fn every_source_execution_reference_component_is_checked_independently() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let original = serde_json::to_value(matching_binding(&route)).unwrap();
    for field in [
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
    ] {
        let mut value = original.clone();
        value[field] = if field.ends_with("revision") {
            serde_json::json!(99)
        } else {
            serde_json::json!("independent-mismatch")
        };
        let binding = serde_json::from_value(value).unwrap();
        assert_eq!(
            check_source_execution_binding(&route, &binding)
                .unwrap_err()
                .code,
            "dcall_source_execution_stale",
            "{field}"
        );
    }
}

#[test]
fn corrupted_persisted_grant_relationships_fail_reload_without_repair() {
    for remote in [false, true] {
        let fixture = build_fixture("fog-01", if remote { "cloud-01" } else { "fog-01" });
        let route = derive(&fixture, "fog-01").unwrap();
        let endpoint = sample_endpoint("cloud-01", 1);
        let mut baseline = DCallGrantStateV1::empty();
        let issued = issue_grant(&mut baseline, route, if remote { DCallDeliveryAuthorityV1::Http(&endpoint) } else { DCallDeliveryAuthorityV1::None }, "m1".into(), "c1".into(), "d".repeat(64), 4, "e".repeat(64), now()).unwrap();
        let mut mutations = vec![
            ("grant_id", serde_json::json!("")),
            ("dgraph_ref.id", serde_json::json!("")), ("dmap_ref.id", serde_json::json!("")),
            ("dgraph_ref.revision", serde_json::json!(0)), ("dmap_ref.revision", serde_json::json!(0)),
            ("dgraph_ref.digest", serde_json::json!("invalid")), ("dmap_ref.digest", serde_json::json!("invalid")),
            ("source_dcode_evidence_sha256", serde_json::json!("invalid")),
            ("source.node_id", serde_json::json!("cloud-01")),
        ];
        let mut mutations: Vec<(String, serde_json::Value)> = mutations.drain(..).map(|(field, value)| (field.to_string(), value)).collect();
        for side in ["source", "destination"] {
            for field in ["service_id", "node_id", "port", "dserv_artifact_id", "dserv_artifact_digest", "dserv_artifact_descriptor_digest"] {
                mutations.push((format!("{side}.{field}"), serde_json::json!("")));
            }
        }
        if remote {
            mutations.extend([
                ("endpoint_id", serde_json::json!("")), ("endpoint_revision", serde_json::json!(0)),
                ("endpoint_descriptor_digest", serde_json::json!("invalid")),
                ("endpoint_id", serde_json::Value::Null), ("endpoint_revision", serde_json::Value::Null),
                ("endpoint_descriptor_digest", serde_json::Value::Null),
            ].into_iter().map(|(field, value)| (field.to_string(), value)));
        } else {
            mutations.push(("endpoint_id".to_string(), serde_json::json!("unexpected")));
        }
        for (field, replacement) in mutations {
            let db = tempfile_db();
            let mut value = serde_json::to_value(&baseline).unwrap();
            let grant = &mut value["grants"][&issued.grant.grant_id]["grant"];
            if let Some((outer, inner)) = field.split_once('.') {
                grant[outer][inner] = replacement;
            } else {
                grant[&field] = replacement;
            }
            let corrupt: DCallGrantStateV1 = serde_json::from_value(value.clone()).unwrap();
            persist_state(&db, &corrupt).unwrap();
            assert!(load_state(&db).is_err(), "remote={remote}, field={field}");
            let stored: serde_json::Value =
                crate::storage::control_state::load_domain_json(&db, DOMAIN_DCALL_GRANT_STATE)
                    .unwrap()
                    .unwrap();
            assert_eq!(stored, value, "reload must not repair {field}");
        }
    }
}

#[test]
fn stale_dgraph_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.dgraph_digest = format!("sha256:{}", "9".repeat(64));
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

#[test]
fn stale_dmap_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.dmap_revision = route.dmap_ref.revision + 1;
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

#[test]
fn stale_source_artifact_id_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.dserv_artifact_id = "dserv:pipeline:some-other-artifact".to_string();
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

#[test]
fn stale_source_module_digest_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.dserv_artifact_digest = "9".repeat(64);
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

#[test]
fn stale_source_descriptor_digest_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.dserv_artifact_descriptor_digest = format!("sha256:{}", "9".repeat(64));
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

#[test]
fn stale_source_node_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.node_id = "some-other-node".to_string();
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

#[test]
fn stale_source_service_binding_is_rejected_before_grant_issuance() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.service_id = "some-other-service".to_string();
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

// ------------------------------------------------------------ PERSISTENCE

fn tempfile_db() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dcall-grant-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("control-state.sqlite3")
}

#[test]
fn redeemed_grant_state_survives_restart_and_cannot_be_replayed() {
    let fixture = build_fixture("fog-01", "fog-01");
    let db_path = tempfile_db();

    let mut grants = load_state(&db_path).unwrap();
    let route = derive(&fixture, "fog-01").unwrap();
    // Independent review v2 (I7): the persisted-state integrity validator
    // now checks payload_sha256's SHAPE (64 lowercase hex chars) on every
    // reload, so this test -- the only one that actually round-trips
    // through `load_state` twice -- needs a shape-valid placeholder digest,
    // unlike the other tests in this file that never reload from disk.
    let payload_sha256 = "d".repeat(64);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        payload_sha256.clone(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    persist_state(&db_path, &grants).unwrap();

    // Reload as if the process restarted, then redeem.
    let mut reloaded = load_state(&db_path).unwrap();
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    redeem_grant(
        &mut reloaded,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        &payload_sha256,
        4,
        None, now(),
    )
        .expect("redemption after reload should succeed");
    persist_state(&db_path, &reloaded).unwrap();

    // Restart AGAIN (fresh load from disk) and attempt to replay the SAME
    // grant/token -- this is the actual "durable across restart" proof: no
    // in-memory-only state is what's preventing the replay here.
    let mut reloaded_again = load_state(&db_path).unwrap();
    let error = redeem_grant(
        &mut reloaded_again,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        &payload_sha256,
        4,
        None, now(),
    )
        .unwrap_err();
    assert!(matches!(error, RedeemError::AlreadyRedeemed));
}

#[test]
fn corrupted_persisted_grant_state_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    // Corrupt the map key so it no longer matches the record's own grant_id.
    let record = grants.grants.remove(&issued.grant.grant_id).unwrap();
    grants.grants.insert("some-other-key".to_string(), record);
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(error.contains("does not match its own grant_id"));
}

/// I7: temporal relationship, not merely parseability -- `expires_at` must
/// be strictly after `issued_at`.
#[test]
fn persisted_grant_with_expires_at_not_after_issued_at_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.expires_at = record.grant.issued_at.clone();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("expires_at <= issued_at"),
        "unexpected error: {error}"
    );
}

/// I7: a `Pending` grant must never carry a `redeemed_at` timestamp.
#[test]
fn persisted_pending_grant_with_a_redeemed_at_timestamp_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.redeemed_at = Some(record.grant.issued_at.clone());
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("is Pending but has a redeemed_at timestamp"),
        "unexpected error: {error}"
    );
}

/// I7: `route_kind=local` must carry NO endpoint fields.
#[test]
fn persisted_local_grant_carrying_endpoint_fields_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.endpoint_id = Some("dcall-endpoint:fog-01:http".to_string());
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("route_kind=local but carries endpoint fields"),
        "unexpected error: {error}"
    );
}

/// I7: `route_kind=remote` must carry ALL endpoint fields.
#[test]
fn persisted_remote_grant_missing_endpoint_fields_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "cloud-01");
    let endpoint = crate::core::dcall_endpoint::DNodeTransportEndpointV1 {
        node_id: "cloud-01".to_string(),
        transport: crate::core::dcall_endpoint::TRANSPORT_HTTP.to_string(),
        endpoint_id: "dcall-endpoint:cloud-01:http".to_string(),
        revision: 1,
        base_url: "http://127.0.0.1:9100".to_string(),
        descriptor_digest: String::new(),
    };
    let mut endpoint = endpoint;
    endpoint.descriptor_digest =
        crate::core::dcall_endpoint::recompute_endpoint_digest(&endpoint).unwrap();
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::Http(&endpoint),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.endpoint_id = None;
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("route_kind=remote but is missing"),
        "unexpected error: {error}"
    );
}

/// I7: an empty required identifier (here, `correlation_id`) fails closed.
#[test]
fn persisted_grant_with_empty_required_identifier_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.correlation_id = String::new();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("empty required identifier"),
        "unexpected error: {error}"
    );
}

// -- J3/J4: shared digest-shape validator + cheap persistence completion ---

#[test]
fn is_valid_sha256_hex_accepts_only_the_exact_bare_lowercase_shape() {
    assert!(is_valid_sha256_hex(&"a".repeat(64)));
    assert!(
        !is_valid_sha256_hex(&"a".repeat(63)),
        "63 chars must be rejected"
    );
    assert!(
        !is_valid_sha256_hex(&"a".repeat(65)),
        "65 chars must be rejected"
    );
    assert!(
        !is_valid_sha256_hex(&"g".repeat(64)),
        "non-hex must be rejected"
    );
    assert!(
        !is_valid_sha256_hex(&"A".repeat(64)),
        "uppercase must be rejected (canonical form is lowercase)"
    );
    assert!(
        !is_valid_sha256_hex(&format!("sha256:{}", "a".repeat(64))),
        "a 'sha256:' prefix is the WRONG form for this bare-hex validator"
    );
}

/// J4: `endpoint_revision` presence alone was not enough -- it must also be
/// `>= 1` (0 is not a valid revision under this codebase's established
/// revision-numbering convention).
#[test]
fn persisted_remote_grant_with_endpoint_revision_zero_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let endpoint = sample_endpoint("cloud-01", 1);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::Http(&endpoint),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.endpoint_revision = Some(0);
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("endpoint_revision") && error.contains("< 1"),
        "unexpected error: {error}"
    );
}

/// J4: `dgraph_ref.digest` must itself be a structurally valid canonical
/// digest, not merely a present string.
#[test]
fn persisted_grant_with_malformed_dgraph_ref_digest_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.dgraph_ref.digest = "not-a-real-digest".to_string();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("dgraph_ref.digest"),
        "unexpected error: {error}"
    );
}

/// J4: `source.dserv_artifact_digest` (bare-hex module digest) must itself
/// be shape-valid.
#[test]
fn persisted_grant_with_malformed_source_module_digest_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.source.dserv_artifact_digest = "too-short".to_string();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("source.dserv_artifact_digest"),
        "unexpected error: {error}"
    );
}

/// J3: a malformed (but non-empty) `payload_sha256` must be rejected at the
/// SAME shape rule the persisted-state validator enforces, so it can never
/// be persisted in the first place. This exercises the shared validator
/// directly (this codebase has no `AppState`-in-test harness precedent to
/// drive the real Axum handler in an isolated unit test -- see `api::dcode`'s
/// own long-standing documented limitation) -- the handler's own use of this
/// exact function is a one-line call, visually auditable at its call site.
#[test]
fn request_grant_payload_sha256_shape_matches_the_persisted_state_rule() {
    assert!(
        !is_valid_sha256_hex("abc"),
        "a short malformed payload_sha256 must fail the same rule reload enforces"
    );
    assert!(is_valid_sha256_hex(&"a".repeat(64)));
}

// ====================================================================
// Phase 113A closure pass 2: common active authority -> DCode/DCall
// production consumers. Same `dgraph()`/`node()`/`source_artifact()`/
// `dest_artifact()`/`derive()`/`matching_binding()` helpers as the DMap/1
// tests above -- only the DMap FORMAT differs, exactly proving the shared
// v1/v2 authority interpretation (`active_placement_authority`) is what
// makes that reuse possible at all.
// ====================================================================

fn build_v2_fixture(source_node: &str, dest_node: &str) -> Fixture {
    let proposal_id = "prop-v2-1".to_string();
    let mut ddeploy = DDeployStateV1::empty();
    let dg = dgraph();
    let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();
    let dcontinuum = crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
        schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:lab".to_string(),
        revision: 1,
        nodes: if source_node == dest_node {
            vec![node(source_node)]
        } else {
            vec![node(source_node), node(dest_node)]
        },
        links: Vec::new(),
    };
    let dcontinuum_digest =
        crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum).unwrap();
    // `CanonicalDMapV2::validate` requires EXACTLY ONE DCall realization per
    // DGraph call ("missing_dcall_realization"), which in turn requires a
    // real DIoT + DIoT placement to reference -- Phase 113A's minimal
    // structurally-valid "one broker DIoT realizes the one D-Call" shape.
    const DIOT_ID: &str = "diot:broker";
    const DIOT_PLACEMENT_ID: &str = "diotp:broker";
    const REALIZATION_INTERFACE: &str = "sensor-bus";
    let dforward = crate::core::canonical_dforward::CanonicalDForwardV1 {
        schema: crate::core::canonical_dforward::CANONICAL_DFORWARD_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dforward_id: "dforward:pipeline".to_string(),
        revision: 1,
        diots: vec![crate::core::canonical_dforward::CanonicalDIoTV1 {
            diot_id: DIOT_ID.to_string(),
            diot_kind: "broker".to_string(),
            artifact_requirement: crate::core::canonical_dforward::CanonicalArtifactRequirementV1 {
                artifact_id: "dserv:pipeline:broker".to_string(),
            },
            provides: vec![
                crate::core::canonical_dforward::CanonicalDForwardInterfaceV1 {
                    interface: REALIZATION_INTERFACE.to_string(),
                },
            ],
            requires: Vec::new(),
        }],
        external_endpoints: Vec::new(),
        chains: Vec::new(),
        support_jobs: Vec::new(),
    };
    let dforward_digest = dforward.digest().unwrap();

    let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
        schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
        proposal_id: proposal_id.clone(),
        project_id: PROJECT.to_string(),
        application_id: APP.to_string(),
        source: ProposalSource::Operator,
        created_at_utc: Utc::now().to_rfc3339(),
        dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dg.dgraph_id.clone(),
            revision: dg.revision,
            digest: dgraph_digest,
        },
        dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dcontinuum.dcontinuum_id.clone(),
            revision: dcontinuum.revision,
            digest: dcontinuum_digest,
        },
        dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dforward.dforward_id.clone(),
            revision: dforward.revision,
            digest: dforward_digest,
        },
        placements: vec![
            crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: SOURCE_SVC.to_string(),
                node_id: source_node.to_string(),
            },
            crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: DEST_SVC.to_string(),
                node_id: dest_node.to_string(),
            },
        ],
        diot_placements: vec![crate::core::canonical_dmap_v2::DIoTPlacementV2 {
            placement_id: DIOT_PLACEMENT_ID.to_string(),
            diot_id: DIOT_ID.to_string(),
            node_id: source_node.to_string(),
            artifact_binding: crate::core::canonical_dmap_v2::DIoTArtifactBindingV2 {
                artifact_id: "dserv:pipeline:broker".to_string(),
                descriptor_digest: format!("sha256:{}", "b".repeat(64)),
                content_digest: None,
            },
        }],
        dcall_realizations: vec![crate::core::canonical_dmap_v2::DCallRealizationV2 {
            call_id: CALL_ID.to_string(),
            realization_kind: "via_diot_pubsub".to_string(),
            diot_placement_id: DIOT_PLACEMENT_ID.to_string(),
            interface: REALIZATION_INTERFACE.to_string(),
            channel: "pipeline/sensor-to-decoder".to_string(),
        }],
        external_port_bindings: Vec::new(),
        expected_active_dmap_revision: None,
        diot_artifact_authority: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                "dserv:pipeline:broker".to_string(),
                crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2 {
                    artifact_id: "dserv:pipeline:broker".to_string(),
                    execution_generation: 1,
                    execution_projection_digest: format!("sha256:{}", "c".repeat(64)),
                },
            );
            m
        },
    };

    // A minimal real ServiceArtifact matching the binding embedded in the
    // proposal above -- `accept_v2_proposal` re-resolves against this exact
    // snapshot and requires it to match, so this fixture only proves the
    // D-Call chain works end to end when artifact authority is unchanged.
    let broker_artifact = crate::core::dcall::tests::minimal_service_artifact_for_diot(
        PROJECT,
        "dserv:pipeline:broker",
        1,
        &format!("sha256:{}", "c".repeat(64)),
    );

    ddeploy
        .create_v2_proposal(proposal, dg.clone(), dcontinuum.clone(), dforward.clone())
        .unwrap();
    ddeploy
        .accept_v2_proposal(
            &proposal_id,
            "acc-v2-1".to_string(),
            "test".to_string(),
            "2026-01-01T00:00:01Z".to_string(),
            &dcontinuum,
            std::slice::from_ref(&broker_artifact),
            None,
        )
        .unwrap();

    let mut dnode = DNodeRegistryStateV1::empty();
    declare_dnode(&mut dnode, node(source_node)).unwrap();
    if dest_node != source_node {
        declare_dnode(&mut dnode, node(dest_node)).unwrap();
    }
    let mut dcode = DCodeArtifactRegistryStateV1::empty();
    declare_dserv_artifact(&mut dcode, source_artifact()).unwrap();
    declare_dserv_artifact(&mut dcode, dest_artifact()).unwrap();

    Fixture {
        ddeploy,
        dnode,
        dcode,
    }
}

/// Phase 113B2 (sections 16/17/26 E-H): the one real, governed change this
/// helper applies at a time (channel/DIoT-placement-node/DForward
/// revision/artifact execution authority), via a SECOND real
/// `create_v2_proposal`/`accept_v2_proposal` cycle against `fixture.ddeploy`
/// -- never a direct field mutation simulating a change. Mirrors
/// `build_v2_fixture`'s own construction exactly except for the one field
/// under test.
struct V2GovernanceMutation<'a> {
    channel: &'a str,
    interface: &'a str,
    diot_node: &'a str,
    dforward_revision: u64,
    execution_generation: u64,
    execution_projection_digest: &'a str,
}

fn accept_v2_governance_mutation(
    fixture: &mut Fixture,
    source_node: &str,
    dest_node: &str,
    mutation: V2GovernanceMutation,
) {
    let proposal_id = "prop-v2-2".to_string();
    let dg = dgraph();
    let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();
    let dcontinuum = crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
        schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:lab".to_string(),
        revision: 1,
        nodes: if source_node == dest_node {
            vec![node(source_node)]
        } else {
            vec![node(source_node), node(dest_node)]
        },
        links: Vec::new(),
    };
    let dcontinuum_digest =
        crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum).unwrap();
    const DIOT_ID: &str = "diot:broker";
    const DIOT_PLACEMENT_ID: &str = "diotp:broker";
    let dforward = crate::core::canonical_dforward::CanonicalDForwardV1 {
        schema: crate::core::canonical_dforward::CANONICAL_DFORWARD_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dforward_id: "dforward:pipeline".to_string(),
        revision: mutation.dforward_revision,
        diots: vec![crate::core::canonical_dforward::CanonicalDIoTV1 {
            diot_id: DIOT_ID.to_string(),
            diot_kind: "broker".to_string(),
            artifact_requirement: crate::core::canonical_dforward::CanonicalArtifactRequirementV1 {
                artifact_id: "dserv:pipeline:broker".to_string(),
            },
            provides: vec![crate::core::canonical_dforward::CanonicalDForwardInterfaceV1 {
                interface: mutation.interface.to_string(),
            }],
            requires: Vec::new(),
        }],
        external_endpoints: Vec::new(),
        chains: Vec::new(),
        support_jobs: Vec::new(),
    };
    let dforward_digest = dforward.digest().unwrap();

    // The proposal/acceptance CAS gate (`accept_v2_proposal`'s own
    // `expected_active_dmap_revision != previous_revision` check) requires
    // this to name the CURRENTLY active revision, not `None` -- a second,
    // real re-acceptance is not a fresh-deployment.
    let current_revision = crate::core::ddeploy::active_placement_authority(
        &fixture.ddeploy,
        PROJECT,
        APP,
    )
    .unwrap()
    .unwrap()
    .revision;

    let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
        schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
        proposal_id: proposal_id.clone(),
        project_id: PROJECT.to_string(),
        application_id: APP.to_string(),
        source: ProposalSource::Operator,
        created_at_utc: Utc::now().to_rfc3339(),
        dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dg.dgraph_id.clone(),
            revision: dg.revision,
            digest: dgraph_digest,
        },
        dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dcontinuum.dcontinuum_id.clone(),
            revision: dcontinuum.revision,
            digest: dcontinuum_digest,
        },
        dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dforward.dforward_id.clone(),
            revision: dforward.revision,
            digest: dforward_digest,
        },
        placements: vec![
            crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: SOURCE_SVC.to_string(),
                node_id: source_node.to_string(),
            },
            crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: DEST_SVC.to_string(),
                node_id: dest_node.to_string(),
            },
        ],
        diot_placements: vec![crate::core::canonical_dmap_v2::DIoTPlacementV2 {
            placement_id: DIOT_PLACEMENT_ID.to_string(),
            diot_id: DIOT_ID.to_string(),
            node_id: mutation.diot_node.to_string(),
            artifact_binding: crate::core::canonical_dmap_v2::DIoTArtifactBindingV2 {
                artifact_id: "dserv:pipeline:broker".to_string(),
                descriptor_digest: format!("sha256:{}", "b".repeat(64)),
                content_digest: None,
            },
        }],
        dcall_realizations: vec![crate::core::canonical_dmap_v2::DCallRealizationV2 {
            call_id: CALL_ID.to_string(),
            realization_kind: "via_diot_pubsub".to_string(),
            diot_placement_id: DIOT_PLACEMENT_ID.to_string(),
            interface: mutation.interface.to_string(),
            channel: mutation.channel.to_string(),
        }],
        external_port_bindings: Vec::new(),
        expected_active_dmap_revision: Some(current_revision),
        diot_artifact_authority: {
            let mut m = std::collections::BTreeMap::new();
            m.insert(
                "dserv:pipeline:broker".to_string(),
                crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2 {
                    artifact_id: "dserv:pipeline:broker".to_string(),
                    execution_generation: mutation.execution_generation,
                    execution_projection_digest: mutation.execution_projection_digest.to_string(),
                },
            );
            m
        },
    };

    let broker_artifact = crate::core::dcall::tests::minimal_service_artifact_for_diot(
        PROJECT,
        "dserv:pipeline:broker",
        mutation.execution_generation,
        mutation.execution_projection_digest,
    );

    fixture
        .ddeploy
        .create_v2_proposal(proposal, dg, dcontinuum.clone(), dforward)
        .unwrap();
    fixture
        .ddeploy
        .accept_v2_proposal(
            &proposal_id,
            "acc-v2-2".to_string(),
            "test".to_string(),
            "2026-01-01T00:00:02Z".to_string(),
            &dcontinuum,
            std::slice::from_ref(&broker_artifact),
            None,
        )
        .unwrap();
}

/// The exact governance-baseline values `build_v2_fixture` itself uses --
/// a mutation test changes exactly ONE of these via
/// `accept_v2_governance_mutation`, keeping the rest identical to the
/// original acceptance.
fn baseline_v2_mutation(source_node: &str) -> V2GovernanceMutation<'_> {
    V2GovernanceMutation {
        channel: "pipeline/sensor-to-decoder",
        interface: "sensor-bus",
        diot_node: source_node,
        dforward_revision: 1,
        execution_generation: 1,
        execution_projection_digest: concat!(
            "sha256:",
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        ),
    }
}

// ---------------------------------------------------- DMAP/2 DCODE (section 14)

#[test]
fn v2_dcode_authorization_succeeds_for_correctly_placed_service() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let auth = authorize_dcode_invocation(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        SOURCE_SVC,
        "fog-01",
    )
    .expect("v2-governed authorization should succeed");
    assert_eq!(auth.node_id, "fog-01");
}

#[test]
fn v2_dcode_authorization_evidence_carries_exact_dmap2_identity() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let active = crate::core::ddeploy::active_placement_authority(&fixture.ddeploy, PROJECT, APP)
        .unwrap()
        .unwrap();
    assert!(active.v2.is_some(), "fixture must be governed under DMap/2");
    let auth = authorize_dcode_invocation(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        SOURCE_SVC,
        "fog-01",
    )
    .unwrap();
    assert_eq!(auth.dmap_ref.id, active.dmap_id);
    assert_eq!(auth.dmap_ref.revision, active.revision);
    assert_eq!(auth.dmap_ref.digest, active.digest);
}

#[test]
fn v2_dcode_authorization_wrong_node_is_rejected() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let error = authorize_dcode_invocation(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        SOURCE_SVC,
        "cloud-99",
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::core::dcode_authorization::F_DCODE_SERVICE_NOT_PLACED_ON_NODE
    );
}

#[test]
fn v2_dcode_authorization_stale_revision_is_rejected() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let auth = authorize_dcode_invocation(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        SOURCE_SVC,
        "fog-01",
    )
    .unwrap();
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
    active.map.revision += 1;
    active.digest = active.map.digest().unwrap();
    let error = crate::core::dcode_authorization::verify_dcode_authorization_still_current(
        &auth,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::core::dcode_authorization::F_DCODE_STALE_AUTHORIZATION
    );
}

#[test]
fn v2_dcode_authorization_stale_digest_is_rejected() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let auth = authorize_dcode_invocation(
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        SOURCE_SVC,
        "fog-01",
    )
    .unwrap();
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
    // Digest drifts while revision stays the same -- an independent
    // staleness dimension from the revision-bump test above.
    active.digest = format!("sha256:{}", "9".repeat(64));
    let error = crate::core::dcode_authorization::verify_dcode_authorization_still_current(
        &auth,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::core::dcode_authorization::F_DCODE_STALE_AUTHORIZATION
    );
}

// ---------------------------------------------------- DMAP/2 ROUTE (section 15)

#[test]
fn v2_route_derives_source_and_destination_from_dgraph_and_dmap2_placement() {
    let fixture = build_v2_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01")
        .expect("route derivation must still succeed under an active DMap/2");
    // Semantic source/target: from DGraph (`dgraph()`'s one call edge,
    // SOURCE_SVC -> DEST_SVC), never invented by this function.
    assert_eq!(route.source.service_id, SOURCE_SVC);
    assert_eq!(route.destination.service_id, DEST_SVC);
    // Placement: from DMap/2's own placements, never caller input (`derive`
    // only ever supplies the SOURCE node -- the destination node is 100%
    // derived below).
    assert_eq!(route.source.node_id, "fog-01");
    assert_eq!(route.destination.node_id, "cloud-01");
    assert_eq!(route.route_kind, ROUTE_KIND_REMOTE);
    // The realization boundary (section 11/12) is recorded on the route but
    // does NOT prevent route derivation itself from succeeding.
    let realization = route
        .v2_dcall_realization
        .as_ref()
        .expect("route governed under active DMap/2 must carry a realization authority");
    assert_eq!(realization.realization_kind, "via_diot_pubsub");
    assert_eq!(realization.diot_placement_id, "diotp:broker");
    assert_eq!(realization.interface, "sensor-bus");
    assert_eq!(realization.channel, "pipeline/sensor-to-decoder");
}

/// Independent review fix (Phase 113B1): an earlier version of
/// `derive_dcall_route` silently FABRICATED `"via_diot_pubsub"` when no
/// `DCallRealizationV2` matched `call_id`, instead of treating that as the
/// persisted-authority inconsistency it actually is (a validated DMap/2 can
/// never legitimately reach this state -- `CanonicalDMapV2::validate`
/// requires exactly one realization per DGraph call). This test forces that
/// otherwise-unreachable state directly on the in-memory active authority
/// (mirroring the stale-authority tests' own technique of mutating
/// `active.map` to simulate persisted-state corruption) and proves route
/// derivation now fails closed instead.
#[test]
fn v2_route_derivation_fails_closed_when_active_dmap2_is_missing_a_realization_for_the_call() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
    active.map.dcall_realizations.clear();
    active.digest = active.map.digest().unwrap();
    let error = derive_dcall_route(
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        PROJECT,
        APP,
        CALL_ID,
        SOURCE_SVC,
        "fog-01",
        OUTPUT_PORT,
    )
    .unwrap_err();
    assert_eq!(error.code, "dcall_v2_realization_missing");
}

#[test]
fn v2_route_carries_exact_dmap2_digest() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let active = crate::core::ddeploy::active_placement_authority(&fixture.ddeploy, PROJECT, APP)
        .unwrap()
        .unwrap();
    let route = derive(&fixture, "fog-01").unwrap();
    assert_eq!(route.dmap_ref.id, active.dmap_id);
    assert_eq!(route.dmap_ref.revision, active.revision);
    assert_eq!(route.dmap_ref.digest, active.digest);
    let realization = route
        .v2_dcall_realization
        .as_ref()
        .expect("same-node route is STILL governed under DMap/2 realization boundary");
    assert_eq!(realization.realization_kind, "via_diot_pubsub");
    let v2_extras = active.v2.as_ref().unwrap();
    assert_eq!(realization.dforward_id, v2_extras.dforward_id);
    assert_eq!(realization.dforward_revision, v2_extras.dforward_revision);
    assert_eq!(realization.dforward_digest, v2_extras.dforward_digest);
}

// ---------------------------------------------------- STALE AUTHORITY (section 16)

/// Scenario A: source-executed under DMap/2 revision N, authority moves to
/// N+1, dispatch of the OLD binding is rejected -- proves K2's source-
/// execution-authority-continuity mechanism (`check_source_execution_binding`)
/// works identically for DMap/2, not only DMap/1 (section 8). Exercises the
/// underlying library check directly; the full production HTTP path
/// additionally refuses ALL DMap/2 dispatch regardless of staleness (section
/// 11/12, proven separately below).
#[test]
fn v2_source_execution_binding_rejects_dispatch_after_authority_moves_to_a_new_revision() {
    let fixture = build_v2_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut binding = matching_binding(&route);
    binding.dmap_revision = route.dmap_ref.revision + 1;
    let error = check_source_execution_binding(&route, &binding).unwrap_err();
    assert_eq!(error.code, "dcall_source_execution_stale");
}

/// Scenario B: a grant issued under DMap/2 authority X, then authority
/// changes BEFORE redemption -- redemption must fail closed rather than
/// trust the grant's stale binding, mirroring
/// `stale_dmap_after_grant_issuance_fails_redemption`'s DMap/1 proof exactly.
#[test]
fn v2_grant_issued_under_stale_authority_fails_redemption_after_activation_changes() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
    active.map.revision += 1;
    active.digest = active.map.digest().unwrap();
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(f) if f.code == "dcall_stale_authority"));
}

/// Scenario C: placement changes (destination moves to a different node)
/// while the semantic D-Call identity is retained -- redemption of the OLD
/// grant must fail closed, mirroring
/// `moved_destination_after_grant_issuance_fails_redemption`'s DMap/1 proof.
#[test]
fn v2_grant_issued_before_a_placement_change_fails_redemption() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
    for placement in active.map.placements.iter_mut() {
        if placement.service_id == DEST_SVC {
            placement.node_id = "cloud-99".to_string();
        }
    }
    active.digest = active.map.digest().unwrap();
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
}

// ---------------------------------------------------- NO HTTP FALLBACK (section 17)
//
// The API-layer fail-closed check (`api::dcall::post_request_grant`, right
// after `derive_dcall_route` succeeds and BEFORE endpoint lookup /
// `check_source_execution_binding` / message-limit checks / `issue_grant`)
// cannot be exercised from this core-module test file without an
// `AppState`-in-test HTTP harness, which this codebase has no precedent for
// (see `request_grant_payload_sha256_shape_matches_the_persisted_state_rule`'s
// own doc comment above for the same long-standing limitation). What CAN be
// proven at this layer -- and is proven here -- is the exact library-level
// signal the handler branches on: a route derived under an active DMap/2
// always carries `v2_dcall_realization = Some(..)`, is structurally
// available BEFORE any endpoint/grant/redemption call the handler would
// otherwise make, and NOTHING in the core route/grant/redeem functions
// themselves ever performs a network send, endpoint lookup, or destination
// D-Code execution as a side effect of route derivation alone.
#[test]
fn v2_route_derivation_alone_signals_the_realization_boundary_before_any_delivery_side_effect() {
    let fixture = build_v2_fixture("fog-01", "cloud-01");
    // Route derivation is a pure computation -- no grant state, no endpoint
    // registry, and no D-Code worker are touched by `derive_dcall_route`
    // itself (its signature takes only read-only state snapshots and
    // returns a plain value).
    let route =
        derive(&fixture, "fog-01").expect("route derivation must succeed and be side-effect-free");
    let realization = route.v2_dcall_realization.as_ref().expect(
        "the handler's fail-closed check keys off exactly this field, checked before it does anything else",
    );
    assert_eq!(realization.realization_kind, "via_diot_pubsub");
    // No grant exists anywhere yet -- deriving a route never inserts one.
    let empty_grants = DCallGrantStateV1::empty();
    assert!(empty_grants.grants.is_empty());
}

#[test]
fn v2_dmap1_route_never_signals_the_realization_boundary() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    assert_eq!(
        route.v2_dcall_realization, None,
        "a DMap/1 active authority must never trigger the DMap/2-only realization boundary"
    );
}

/// Phase 113A closure pass 3: a minimal, structurally-valid, real
/// `ServiceArtifact` (native_process runtime -- avoids container-only
/// required fields) used only to give `resolve_diot_artifact_authority` a
/// real record to resolve against in this module's own D-Call fixtures.
/// `pub(crate)` so `build_v2_fixture` (this file) can construct a snapshot
/// matching whatever `DIoTArtifactAuthorityBindingV2` it embedded in a v2
/// proposal.
pub(crate) fn minimal_service_artifact_for_diot(
    project_id: &str,
    artifact_id: &str,
    execution_generation: u64,
    execution_projection_digest: &str,
) -> crate::core::service_artifact_registry::ServiceArtifact {
    use crate::core::service_artifact_registry::*;
    ServiceArtifact {
        schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
        artifact_id: artifact_id.to_string(),
        service_id: artifact_id.to_string(),
        runtime: ServiceRuntimeSpec {
            kind: "native_process".to_string(),
            container: None,
            native_process: Some(NativeProcessDeploymentSpecV1 {
                process_name: "broker".to_string(),
                entrypoint: "bin/broker-run".to_string(),
                args: Vec::new(),
                working_directory: None,
            }),
        },
        probes: ServiceProbeSet {
            execution: ServiceProbeSpec {
                kind: "process_running".to_string(),
                params: BTreeMap::new(),
                vantage_point: Some("local_host".to_string()),
            },
            health: None,
            availability: None,
        },
        metadata: None,
        control_plane: ServiceArtifactControlPlane {
            project_id: project_id.to_string(),
            execution_generation,
            execution_projection_digest: execution_projection_digest.to_string(),
            ..Default::default()
        },
    }
}

// ====================================================================
// Phase 113B2 (ADR-0023): D-Call grant authority for exact DMap/2
// realization + exact DIoT runtime binding. Sections 15-17, 25-28.
// ====================================================================

fn issue_v2_grant(
    source_node: &str,
    dest_node: &str,
) -> (
    Fixture,
    DCallGrantStateV1,
    IssuedGrant,
    DIoTRuntimeBindingRegistryStateV1,
) {
    let fixture = build_v2_fixture(source_node, dest_node);
    let route = derive(&fixture, source_node).unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    (fixture, grants, issued, diot_bindings)
}

// -------------------------------------------------- GRANT ISSUANCE (section 25)

/// 25A: DMap/2 + a current DIoT runtime binding succeeds.
#[test]
fn v2_grant_issuance_succeeds_when_diot_runtime_binding_is_current() {
    let (_fixture, grants, issued, _diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    assert!(grants.grants.contains_key(&issued.grant.grant_id));
    assert!(issued.grant.v2_dcall_transport_authority.is_some());
}

/// 25B: no DIoT runtime binding is registered for the realization's
/// placement -- this is the exact condition `post_request_grant` branches
/// on (`diot_runtime_binding_state.get(...).ok_or_else(...)`, yielding
/// `dcall_diot_runtime_binding_missing`) BEFORE any grant is constructed.
/// No AppState-in-test HTTP harness exists in this codebase (see
/// `request_grant_payload_sha256_shape_matches_the_persisted_state_rule`'s
/// own doc comment for the same long-standing limitation), so this proves
/// the library-level fact the handler relies on.
#[test]
fn v2_grant_issuance_fails_closed_when_diot_runtime_binding_is_missing() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let realization = route.v2_dcall_realization.as_ref().unwrap();
    let diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    assert!(
        diot_bindings
            .get(PROJECT, APP, &realization.diot_placement_id)
            .is_none(),
        "no binding was ever registered for this placement"
    );
    // `issue_grant` itself also refuses to construct a grant for this route
    // without a `DiotPubsub` authority -- zero grant is ever inserted.
    let mut grants = DCallGrantStateV1::empty();
    let error = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        "dcall_v2_realization_requires_diot_transport_authority"
    );
    assert!(grants.grants.is_empty());
}

/// 25C: the DIoT runtime binding exists but is stale (DMap authority moved
/// since it was registered) -- `verify_runtime_binding_still_current`
/// (the ONE Phase 113B1 validator `post_request_grant` reuses) must reject
/// it BEFORE any grant is issued.
#[test]
fn v2_grant_issuance_fails_closed_when_diot_runtime_binding_is_stale() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let binding = crate::core::diot_runtime_binding::register_runtime_binding(
        &mut diot_bindings,
        &fixture.ddeploy,
        &fixture.dnode,
        PROJECT,
        APP,
        &route.v2_dcall_realization.as_ref().unwrap().diot_placement_id,
        "mqtt://broker.lab:1883",
        "2026-01-01T00:00:00Z",
    )
    .unwrap();
    let mut ddeploy = fixture.ddeploy.clone();
    let key = application_key(PROJECT, APP);
    let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
    active.map.revision += 1;
    active.digest = active.map.digest().unwrap();
    let error =
        verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
    assert_eq!(error.code, "diot_runtime_binding_stale");
    // Zero grant was ever inserted for this stale binding -- there is no
    // grant-issuance call at all in this scenario, matching what
    // `post_request_grant` does: reject before constructing the grant.
}

/// 25D (section 11): a SAME-NODE DMap/2 call still receives typed DIoT
/// transport authority -- it never becomes a bare local grant merely
/// because `source.node_id == destination.node_id`.
#[test]
fn v2_grant_issuance_for_same_node_call_still_carries_diot_transport_authority() {
    let (_fixture, _grants, issued, _diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    assert_eq!(issued.grant.route_kind, ROUTE_KIND_LOCAL);
    assert!(
        issued.grant.v2_dcall_transport_authority.is_some(),
        "a same-node DMap/2 via_diot_pubsub call must still carry DIoT transport authority"
    );
}

/// 25E (section 10): a REMOTE DMap/2 call succeeds with ZERO HTTP
/// transport endpoint ever declared or consulted.
#[test]
fn v2_grant_issuance_for_remote_call_succeeds_with_zero_http_endpoint() {
    let (_fixture, _grants, issued, _diot_bindings) = issue_v2_grant("fog-01", "cloud-01");
    assert_eq!(issued.grant.route_kind, ROUTE_KIND_REMOTE);
    assert!(issued.grant.v2_dcall_transport_authority.is_some());
    assert!(issued.grant.endpoint_id.is_none());
    assert!(issued.grant.endpoint_revision.is_none());
    assert!(issued.grant.endpoint_descriptor_digest.is_none());
}

/// 25F (section 10): `issue_grant`'s `DCallDeliveryAuthorityV1` enum makes
/// it IMPOSSIBLE to construct a grant carrying both HTTP endpoint authority
/// and DIoT transport authority at once -- a `DiotPubsub` grant's endpoint
/// fields are always `None`, regardless of whether an HTTP endpoint happens
/// to be declared for that same destination node elsewhere in the system.
#[test]
fn v2_grant_never_carries_http_endpoint_authority_even_when_one_exists_for_the_destination_node() {
    let fixture = build_v2_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    // An HTTP endpoint IS declared for the destination node, in a completely
    // separate registry -- `issue_grant` is never even given a way to
    // consult it while also carrying `DiotPubsub` authority.
    let endpoint = DNodeTransportEndpointV1 {
        endpoint_id: "endpoint:cloud-01:http".to_string(),
        node_id: "cloud-01".to_string(),
        transport: TRANSPORT_HTTP.to_string(),
        base_url: "http://cloud-01.lab:9000".to_string(),
        revision: 1,
        descriptor_digest: format!("sha256:{}", "9".repeat(64)),
    };
    let _ = &endpoint; // never passed to issue_grant below -- see doc comment.
    let mut grants = DCallGrantStateV1::empty();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    assert!(issued.grant.endpoint_id.is_none());
    assert!(issued.grant.endpoint_revision.is_none());
    assert!(issued.grant.endpoint_descriptor_digest.is_none());
}

/// 25G: the grant's bound realization fields exactly equal the currently
/// active DMap/2's own realization -- never independently re-derived.
#[test]
fn v2_grant_realization_fields_exactly_equal_active_dmap2() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let expected = route.v2_dcall_realization.clone().unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let bound = issued.grant.v2_dcall_transport_authority.unwrap();
    assert_eq!(bound.realization_kind, expected.realization_kind);
    assert_eq!(bound.diot_placement_id, expected.diot_placement_id);
    assert_eq!(bound.interface, expected.interface);
    assert_eq!(bound.channel, expected.channel);
    assert_eq!(bound.dforward_id, expected.dforward_id);
    assert_eq!(bound.dforward_revision, expected.dforward_revision);
    assert_eq!(bound.dforward_digest, expected.dforward_digest);
}

/// 25H: the grant's bound binding identity exactly equals the CURRENT
/// binding it was issued against.
#[test]
fn v2_grant_binding_fields_exactly_equal_current_binding() {
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let placement_id = route.v2_dcall_realization.as_ref().unwrap().diot_placement_id.clone();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let current_binding = diot_bindings.get(PROJECT, APP, &placement_id).unwrap().clone();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let bound = issued.grant.v2_dcall_transport_authority.unwrap();
    assert_eq!(bound.binding_id, current_binding.binding_id);
    assert_eq!(bound.binding_revision, current_binding.binding_revision);
    assert_eq!(bound.descriptor_digest, current_binding.descriptor_digest);
}

/// 25I: DMap/1 local delivery is unchanged -- the grant carries no DIoT
/// transport authority at all.
#[test]
fn v2_dmap1_local_grant_carries_no_diot_transport_authority() {
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut grants = DCallGrantStateV1::empty();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    assert!(issued.grant.v2_dcall_transport_authority.is_none());
    assert_eq!(issued.grant.route_kind, ROUTE_KIND_LOCAL);
}

/// 25J: DMap/1 remote (HTTP) delivery is unchanged -- the grant carries
/// endpoint authority and no DIoT transport authority.
#[test]
fn v2_dmap1_remote_grant_carries_endpoint_authority_and_no_diot_transport_authority() {
    let fixture = build_fixture("fog-01", "cloud-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let endpoint = DNodeTransportEndpointV1 {
        endpoint_id: "endpoint:cloud-01:http".to_string(),
        node_id: "cloud-01".to_string(),
        transport: TRANSPORT_HTTP.to_string(),
        base_url: "http://cloud-01.lab:9000".to_string(),
        revision: 1,
        descriptor_digest: format!("sha256:{}", "9".repeat(64)),
    };
    let mut grants = DCallGrantStateV1::empty();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::Http(&endpoint),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    assert!(issued.grant.v2_dcall_transport_authority.is_none());
    assert_eq!(issued.grant.endpoint_id.as_deref(), Some("endpoint:cloud-01:http"));
}

/// 25K (section 12): `issue_grant` rejects every impossible authority
/// combination via its typed `DCallDeliveryAuthorityV1` design.
#[test]
fn v2_issue_grant_rejects_impossible_authority_combinations() {
    // DMap/1 route + DIoT transport authority.
    let v1_fixture = build_fixture("fog-01", "fog-01");
    let v1_route = derive(&v1_fixture, "fog-01").unwrap();
    let bogus_authority = DCallRealizationTransportAuthorityV1 {
        realization_kind: "via_diot_pubsub".to_string(),
        diot_placement_id: "diotp:x".to_string(),
        interface: "iface".to_string(),
        channel: "chan".to_string(),
        dforward_id: "dforward:x".to_string(),
        dforward_revision: 1,
        dforward_digest: format!("sha256:{}", "a".repeat(64)),
        binding_id: "diot-runtime-binding:x".to_string(),
        binding_revision: 1,
        descriptor_digest: format!("sha256:{}", "b".repeat(64)),
        transport: "mqtt".to_string(),
    };
    let mut grants = DCallGrantStateV1::empty();
    let error = issue_grant(
        &mut grants,
        v1_route,
        DCallDeliveryAuthorityV1::DiotPubsub(bogus_authority),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap_err();
    assert_eq!(error.code, "dcall_v1_route_cannot_carry_diot_transport_authority");
    assert!(grants.grants.is_empty());

    // DMap/2 via_diot_pubsub route + HTTP endpoint authority instead of DIoT
    // transport authority (already proven missing entirely above by
    // `v2_grant_issuance_fails_closed_when_diot_runtime_binding_is_missing`).
    let v2_fixture = build_v2_fixture("fog-01", "cloud-01");
    let v2_route = derive(&v2_fixture, "fog-01").unwrap();
    let endpoint = DNodeTransportEndpointV1 {
        endpoint_id: "endpoint:cloud-01:http".to_string(),
        node_id: "cloud-01".to_string(),
        transport: TRANSPORT_HTTP.to_string(),
        base_url: "http://cloud-01.lab:9000".to_string(),
        revision: 1,
        descriptor_digest: format!("sha256:{}", "9".repeat(64)),
    };
    let mut grants2 = DCallGrantStateV1::empty();
    let error2 = issue_grant(
        &mut grants2,
        v2_route,
        DCallDeliveryAuthorityV1::Http(&endpoint),
        "m1".to_string(),
        "c1".to_string(),
        "deadbeef".to_string(),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap_err();
    assert_eq!(error2.code, "dcall_v2_realization_requires_diot_transport_authority");
    assert!(grants2.grants.is_empty());
}

// -------------------------------------------------- REDEMPTION (section 26)

/// 26A: unchanged authority and unchanged binding redeems normally.
#[test]
fn v2_redemption_succeeds_when_authority_and_binding_are_unchanged() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let v2_authority = issued
        .grant
        .v2_dcall_transport_authority
        .as_ref()
        .expect("a v2 grant must carry transport authority");
    let observed_topic = crate::core::dcall_mqtt_topic::request_topic(
        &issued.grant.project_id,
        &issued.grant.application_id,
        &v2_authority.diot_placement_id,
        &issued.grant.destination.node_id,
        &v2_authority.channel,
    );
    let grant = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        Some(&observed_topic), now(),
    )
    .expect("unchanged v2 authority + binding must redeem");
    assert!(grant.v2_dcall_transport_authority.is_some());
}

/// Phase 113B3 (section 17): the destination's OBSERVED MQTT topic is
/// REQUIRED for a DMap/2 grant and must exactly match DServer's own
/// independent recomputation from the grant's bound identity -- a caller
/// cannot substitute an arbitrary topic and still redeem.
#[test]
fn v2_redemption_requires_observed_mqtt_topic_and_rejects_a_missing_one() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None,
        now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::ObservedTopicMissing));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// Phase 113B3 (section 17): a wrong observed MQTT topic fails redemption
/// and leaves the grant `Pending` -- never consumed by a bad observation.
#[test]
fn v2_redemption_rejects_a_wrong_observed_mqtt_topic_and_leaves_grant_pending() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let wrong_topic = "datum/v1/dcall/deadbeef/deadbeef/deadbeef/deadbeef/deadbeef".to_string();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        Some(&wrong_topic),
        now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::ObservedTopicMismatch));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 113B3.1 section 11B: the test above uses a topic that is wrong in EVERY
/// segment. This isolates the specific case of only the FINAL segment
/// (the hashed `channel`) disagreeing -- every upstream namespace segment
/// (project/application/placement/destination-node) still matches exactly --
/// proving the comparison is a full-topic equality check, not merely a
/// namespace-prefix check that would let a caller substitute an arbitrary
/// channel while keeping the correct upstream prefix.
#[test]
fn v2_redemption_rejects_an_observed_topic_with_only_the_final_channel_segment_wrong() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let v2_authority = issued
        .grant
        .v2_dcall_transport_authority
        .as_ref()
        .expect("a v2 grant must carry transport authority");
    let correct_topic = crate::core::dcall_mqtt_topic::request_topic(
        &issued.grant.project_id,
        &issued.grant.application_id,
        &v2_authority.diot_placement_id,
        &issued.grant.destination.node_id,
        &v2_authority.channel,
    );
    let replacement = "0".repeat(64);
    let mut segments: Vec<&str> = correct_topic.split('/').collect();
    let last = segments.len() - 1;
    segments[last] = replacement.as_str();
    let wrong_final_segment_topic = segments.join("/");
    assert_ne!(wrong_final_segment_topic, correct_topic);
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        Some(&wrong_final_segment_topic),
        now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::ObservedTopicMismatch));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 113B3.1 section 11C: the mirror case -- only an UPSTREAM namespace
/// segment (here, the hashed `project_id`, the first segment) disagrees,
/// while the final (channel) segment is correct. Proves the namespace
/// segments are independently load-bearing too, not merely validated as a
/// group with the final segment.
#[test]
fn v2_redemption_rejects_an_observed_topic_with_only_an_upstream_namespace_segment_wrong() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let v2_authority = issued
        .grant
        .v2_dcall_transport_authority
        .as_ref()
        .expect("a v2 grant must carry transport authority");
    let correct_topic = crate::core::dcall_mqtt_topic::request_topic(
        &issued.grant.project_id,
        &issued.grant.application_id,
        &v2_authority.diot_placement_id,
        &issued.grant.destination.node_id,
        &v2_authority.channel,
    );
    let replacement = "0".repeat(64);
    let mut segments: Vec<&str> = correct_topic.split('/').collect();
    // Segment index 2 is the first hashed identifier (project_id) -- indices
    // 0/1 are the literal "datum"/"v1" prefix, 2..=6 are the five hashed
    // components ending in the channel hash at index 6.
    segments[2] = replacement.as_str();
    let wrong_namespace_segment_topic = segments.join("/");
    assert_ne!(wrong_namespace_segment_topic, correct_topic);
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        Some(&wrong_namespace_segment_topic),
        now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::ObservedTopicMismatch));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 26B: the DIoT runtime binding is re-registered at a NEW revision (a real
/// broker_uri change through the production registration path) after the
/// grant was issued -- redemption must reject the now-stale binding
/// identity.
#[test]
fn v2_redemption_fails_when_binding_revision_changes_after_issuance() {
    let (fixture, mut grants, issued, mut diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    crate::core::diot_runtime_binding::register_runtime_binding(
        &mut diot_bindings,
        &fixture.ddeploy,
        &fixture.dnode,
        PROJECT,
        APP,
        "diotp:broker",
        "mqtt://broker2.lab:1883",
        "2026-01-01T00:00:05Z",
    )
    .expect("re-registration with a changed broker_uri is a real, legitimate revision bump");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(f) if f.code == "dcall_stale_authority"));
    // The grant must remain Pending, not consumed.
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 26B (idempotency counterpart, section 15): an IDENTICAL re-registration
/// (same broker_uri, same authority) is a genuine no-op per
/// `register_runtime_binding`'s own idempotency guarantee -- the binding
/// keeps its original revision, so redemption of a grant issued against it
/// still succeeds.
#[test]
fn v2_redemption_still_succeeds_after_an_idempotent_identical_reregistration() {
    let (fixture, mut grants, issued, mut diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let reregistered = crate::core::diot_runtime_binding::register_runtime_binding(
        &mut diot_bindings,
        &fixture.ddeploy,
        &fixture.dnode,
        PROJECT,
        APP,
        "diotp:broker",
        "mqtt://broker.lab:1883",
        "2026-01-01T00:00:05Z",
    )
    .unwrap();
    let bound = issued.grant.v2_dcall_transport_authority.as_ref().unwrap();
    assert_eq!(
        reregistered.binding_revision, bound.binding_revision,
        "an identical re-registration must not bump the revision"
    );
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let observed_topic = crate::core::dcall_mqtt_topic::request_topic(
        &issued.grant.project_id,
        &issued.grant.application_id,
        &bound.diot_placement_id,
        &issued.grant.destination.node_id,
        &bound.channel,
    );
    redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        Some(&observed_topic), now(),
    )
    .expect("redemption must still succeed after a genuinely idempotent re-registration");
}

/// 26C: the binding's own persisted `descriptor_digest` is tampered with
/// directly (simulating storage-layer corruption, mirroring this codebase's
/// established precedent of direct-mutation for CORRUPTION scenarios --
/// distinct from the GOVERNANCE-mutation tests below, which all go through
/// the real create/accept path). Even though `binding_id`/`binding_revision`
/// still match the grant, the digest mismatch must still be caught.
#[test]
fn v2_redemption_fails_when_binding_descriptor_digest_is_tampered() {
    let (fixture, mut grants, issued, mut diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let key = crate::core::diot_runtime_binding::binding_key(PROJECT, APP, "diotp:broker");
    diot_bindings.bindings.get_mut(&key).unwrap().descriptor_digest =
        format!("sha256:{}", "0".repeat(64));
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 26D: the currently active DMap/2 moves to a new revision (a real second
/// acceptance, even with otherwise-identical governed content) after the
/// grant was issued -- redemption must reject.
#[test]
fn v2_redemption_fails_when_dmap_revision_changes_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", baseline_v2_mutation("fog-01"));
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
}

/// 26E (section 16/17): the realization's CHANNEL changes via a real,
/// governed second acceptance -- redemption of the OLD grant must fail.
#[test]
fn v2_redemption_fails_when_realization_channel_changes_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.channel = "pipeline/sensor-to-decoder/v2";
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(f) if f.code == "dcall_stale_authority"));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 113B2.1 section 12: the realization's INTERFACE changes via a real,
/// governed second acceptance (the DForward's own provided interface and
/// the DCall realization's declared interface are changed together, since
/// canonical validation requires them to agree -- an isolated one-sided
/// interface mutation is not a constructible legitimate proposal) --
/// redemption of the OLD grant must fail and remain Pending. Distinct from
/// the channel-change test above: this exercises the `interface` field of
/// the stale-realization comparison in `redeem_grant`, never previously
/// covered by a dedicated regression.
#[test]
fn v2_redemption_fails_when_realization_interface_changes_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.interface = "sensor-bus-v2";
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(f) if f.code == "dcall_stale_authority"));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 26F: the DIoT placement's NODE changes via a real governed acceptance --
/// redemption of the OLD grant must fail.
#[test]
fn v2_redemption_fails_when_realization_placement_moves_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "cloud-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.diot_node = "cloud-01";
    accept_v2_governance_mutation(&mut fixture, "fog-01", "cloud-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "cloud-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
}

/// 26G: the governing DForward's own revision changes via a real governed
/// acceptance -- redemption of the OLD grant must fail.
#[test]
fn v2_redemption_fails_when_dforward_authority_changes_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.dforward_revision = 2;
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
}

/// 113B2.1 sections 15/16: the two ServiceArtifact execution-authority
/// fields are DISTINCT authority dimensions per `verify_runtime_binding_
/// still_current` and must each get an ISOLATED dedicated test -- a
/// combined test that changes both at once cannot prove either one alone
/// is actually load-bearing. This test isolates `execution_generation`
/// ONLY (`execution_projection_digest` held at its baseline value).
#[test]
fn v2_redemption_fails_when_artifact_execution_generation_changes_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.execution_generation = 2;
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 113B2.1 sections 15/16 (isolated counterpart): `execution_projection_
/// digest` ONLY changes (`execution_generation` held at its baseline
/// value) -- a real content-projection change without a generation bump.
/// Deliberately NOT the same scenario as the generation-change test above.
#[test]
fn v2_redemption_fails_when_artifact_execution_projection_digest_changes_after_issuance() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.execution_projection_digest =
        concat!("sha256:", "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd");
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 26I: every stale-authority rejection above leaves the grant `Pending`,
/// never consumes it -- proven once more explicitly here with the
/// DForward-authority-change scenario, then confirming a subsequent
/// redemption attempt with CORRECT current authority is still impossible
/// because the underlying route itself no longer matches (the grant is
/// unredeemable, not merely "not yet redeemed").
#[test]
fn v2_stale_redemption_leaves_grant_pending_not_redeemed() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let mut mutation = baseline_v2_mutation("fog-01");
    mutation.channel = "pipeline/sensor-to-decoder/moved";
    accept_v2_governance_mutation(&mut fixture, "fog-01", "fog-01", mutation);
    let endpoints = DCallEndpointRegistryStateV1::empty();
    redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    let record = grants.grants.get(&issued.grant.grant_id).unwrap();
    assert_eq!(record.status, DCallGrantStatus::Pending);
    assert!(record.redeemed_at.is_none());
}

/// 113B2.1 section 17 (third sub-item): CURRENT active authority for the
/// application "switches to DMap/1". This codebase has no first-class
/// "downgrade v2 back to v1" governance transition -- `accept_proposal`
/// (v1) never reads or clears `phase113_v2` state, so calling it for an
/// application that already has active v2 authority does not produce a
/// clean switch; it produces the SAME "dual active DMap authority"
/// condition `active_placement_authority`'s own guard is built to detect
/// (see `ddeploy.rs`'s dual-authority reject, already covered by its own
/// dedicated tests in that module). This test proves the practically
/// relevant property instead: redemption of the OLD v2 grant still fails
/// closed (via `authorize_dcode_invocation`'s `F_DCODE_APPLICATION_NOT_
/// ACTIVE`, surfaced here as `StaleAuthority`) rather than silently
/// succeeding or panicking, and the grant remains Pending. It does NOT
/// exercise a "clean v2-to-v1 switch" because the codebase provides no
/// legitimate governance path to produce one without redesigning the
/// dual-authority model, which is explicitly out of scope for this pass.
#[test]
fn v2_redemption_fails_closed_when_a_v1_authority_is_also_registered_for_the_application() {
    let (mut fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let proposal_id = "prop-v1-shadow".to_string();
    let dg = dgraph();
    let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();
    fixture.ddeploy.proposals.insert(
        proposal_id.clone(),
        ProposalRecord {
            proposal: DDeployProposalV1 {
                schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
                proposal_id: proposal_id.clone(),
                project_id: PROJECT.to_string(),
                application_id: APP.to_string(),
                source: ProposalSource::Operator,
                created_at_utc: "2026-01-01T00:00:03Z".to_string(),
                expected_active_dmap_revision: None,
                dgraph_ref: CanonicalContractReferenceV1 {
                    id: dg.dgraph_id.clone(),
                    revision: dg.revision,
                    digest: dgraph_digest.clone(),
                },
                dcontinuum_ref: CanonicalContractReferenceV1 {
                    id: "dcontinuum:lab".to_string(),
                    revision: 1,
                    digest: format!("sha256:{}", "1".repeat(64)),
                },
                placements: vec![
                    CanonicalDServPlacementV1 {
                        service_id: SOURCE_SVC.to_string(),
                        node_id: "fog-01".to_string(),
                    },
                    CanonicalDServPlacementV1 {
                        service_id: DEST_SVC.to_string(),
                        node_id: "fog-01".to_string(),
                    },
                ],
            },
            planner_provenance: None,
            proposal_digest: String::new(),
            dgraph_snapshot: dg.clone(),
            dcontinuum_snapshot: crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
                schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
                project_id: PROJECT.to_string(),
                dcontinuum_id: "dcontinuum:lab".to_string(),
                revision: 1,
                nodes: vec![node("fog-01")],
                links: Vec::new(),
            },
            state: ProposalLifecycleState::Accepted,
            accepted_acceptance_id: Some("acc-v1-shadow".to_string()),
        },
    );
    let dmap = CanonicalDMapV1 {
        schema: CANONICAL_DMAP_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        application_id: APP.to_string(),
        dmap_id: canonical_dmap_id(PROJECT, APP),
        revision: 1,
        dgraph_ref: CanonicalContractReferenceV1 {
            id: dg.dgraph_id.clone(),
            revision: dg.revision,
            digest: dgraph_digest,
        },
        dcontinuum_ref: CanonicalContractReferenceV1 {
            id: "dcontinuum:lab".to_string(),
            revision: 1,
            digest: format!("sha256:{}", "1".repeat(64)),
        },
        placements: vec![
            CanonicalDServPlacementV1 {
                service_id: SOURCE_SVC.to_string(),
                node_id: "fog-01".to_string(),
            },
            CanonicalDServPlacementV1 {
                service_id: DEST_SVC.to_string(),
                node_id: "fog-01".to_string(),
            },
        ],
    };
    let dmap_digest = recompute_dmap_digest(&dmap).unwrap();
    let mut operational_services = BTreeMap::new();
    for svc in [SOURCE_SVC, DEST_SVC] {
        operational_services.insert(
            svc.to_string(),
            OperationalDesiredService {
                service_id: svc.to_string(),
                runtime: "wasm".to_string(),
                desired_state: "running".to_string(),
            },
        );
    }
    fixture.ddeploy.active_by_application.insert(
        application_key(PROJECT, APP),
        ActiveApplicationDeployment {
            application_id: APP.to_string(),
            active_dmap: dmap,
            dmap_digest,
            proposal_id,
            acceptance_id: "acc-v1-shadow".to_string(),
            accepted_at_utc: "2026-01-01T00:00:04Z".to_string(),
            operational_services,
        },
    );

    let endpoints = DCallEndpointRegistryStateV1::empty();
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::StaleAuthority(_)));
    assert_eq!(
        grants.grants.get(&issued.grant.grant_id).unwrap().status,
        DCallGrantStatus::Pending
    );
}

/// 26J: a successful v2 redemption is still single-use.
#[test]
fn v2_redemption_is_single_use() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let v2_authority = issued
        .grant
        .v2_dcall_transport_authority
        .as_ref()
        .expect("a v2 grant must carry transport authority");
    let observed_topic = crate::core::dcall_mqtt_topic::request_topic(
        &issued.grant.project_id,
        &issued.grant.application_id,
        &v2_authority.diot_placement_id,
        &issued.grant.destination.node_id,
        &v2_authority.channel,
    );
    redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        Some(&observed_topic), now(),
    )
    .expect("first redemption should succeed");
    let error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(error, RedeemError::AlreadyRedeemed));
}

/// 26K: the pre-existing token/payload integrity checks are unaffected by
/// the new v2 authority path -- a v2 grant still rejects a wrong token and
/// a payload digest/size mismatch exactly like a v1 grant.
#[test]
fn v2_redemption_rejects_wrong_token_and_payload_mismatch() {
    let (fixture, mut grants, issued, diot_bindings) = issue_v2_grant("fog-01", "fog-01");
    let endpoints = DCallEndpointRegistryStateV1::empty();
    let wrong_token_error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        "wrong-token",
        "fog-01",
        "deadbeef",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(wrong_token_error, RedeemError::TokenInvalid));
    let payload_mismatch_error = redeem_grant(
        &mut grants,
        &fixture.ddeploy,
        &fixture.dnode,
        &fixture.dcode,
        &endpoints,
        &diot_bindings,
        &issued.grant.grant_id,
        &issued.token,
        "fog-01",
        "wrongpayload",
        4,
        None, now(),
    )
    .unwrap_err();
    assert!(matches!(payload_mismatch_error, RedeemError::PayloadMismatch));
}

// -------------------------------------------------- RELOAD / INTEGRITY (section 28)

/// 28A: a HISTORICAL DMap/1 grant record persisted before this field
/// existed (its JSON literally has no `v2_dcall_transport_authority` key)
/// still reloads -- `#[serde(default)]` yields `None`, never a fabricated
/// authority.
#[test]
fn persisted_historical_dmap1_grant_without_the_v2_field_still_reloads() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::None,
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let mut value = serde_json::to_value(&grants).unwrap();
    // Simulate a record persisted BEFORE this field existed: strip the key
    // entirely rather than merely setting it null.
    value["grants"][&issued.grant.grant_id]["grant"]
        .as_object_mut()
        .unwrap()
        .remove("v2_dcall_transport_authority");
    let historical_json = serde_json::to_string(&value).unwrap();
    assert!(!historical_json.contains("v2_dcall_transport_authority"));
    let reloaded: DCallGrantStateV1 = serde_json::from_str(&historical_json)
        .expect("a historical record missing this field entirely must still deserialize");
    validate_persisted_dcall_grant_state_integrity(&reloaded)
        .expect("a historical DMap/1 grant record must still pass integrity validation");
    assert_eq!(
        reloaded
            .grants
            .get(&issued.grant.grant_id)
            .unwrap()
            .grant
            .v2_dcall_transport_authority,
        None
    );
}

/// 28B: a DMap/2 grant with its exact typed transport authority round-trips
/// through real persistence and reload unchanged.
#[test]
fn persisted_dmap2_grant_with_typed_transport_authority_reloads() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let expected = authority.clone();
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    persist_state(&db_path, &grants).unwrap();
    let reloaded = load_state(&db_path).unwrap();
    let bound = reloaded
        .grants
        .get(&issued.grant.grant_id)
        .unwrap()
        .grant
        .v2_dcall_transport_authority
        .clone()
        .expect("DMap/2 grant must reload with its typed transport authority intact");
    assert_eq!(bound, expected);
}

/// 28C: a persisted grant with a MALFORMED typed transport authority
/// (wildcard channel) fails integrity on reload.
#[test]
fn persisted_grant_with_malformed_v2_transport_authority_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record
        .grant
        .v2_dcall_transport_authority
        .as_mut()
        .unwrap()
        .channel = "pipeline/#".to_string();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(error.contains("wildcard"), "unexpected error: {error}");
}

/// 113B2.1 section 5B: a persisted grant whose typed transport authority
/// carries a tampered `realization_kind` (not `via_diot_pubsub`) fails
/// integrity on reload. Distinct from the wildcard-channel test above --
/// isolates the `realization_kind` field of the same production check.
#[test]
fn persisted_grant_with_tampered_realization_kind_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record
        .grant
        .v2_dcall_transport_authority
        .as_mut()
        .unwrap()
        .realization_kind = "via_http".to_string();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("realization_kind"),
        "unexpected error: {error}"
    );
}

/// 113B2.1 section 5D: a persisted grant whose typed transport authority
/// carries a shape-invalid `dforward_digest` (not the canonical
/// `sha256:<64 lowercase hex>` form) fails integrity on reload.
#[test]
fn persisted_grant_with_malformed_dforward_digest_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record
        .grant
        .v2_dcall_transport_authority
        .as_mut()
        .unwrap()
        .dforward_digest = "not-a-real-digest".to_string();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("dforward_digest"),
        "unexpected error: {error}"
    );
}

/// 113B2.1 section 5E: a persisted grant whose typed transport authority
/// carries `binding_revision == 0` (below the minimum valid revision)
/// fails integrity on reload.
#[test]
fn persisted_grant_with_binding_revision_zero_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record
        .grant
        .v2_dcall_transport_authority
        .as_mut()
        .unwrap()
        .binding_revision = 0;
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("binding_revision"),
        "unexpected error: {error}"
    );
}

/// 28D: a persisted grant carrying BOTH HTTP endpoint authority AND DIoT
/// transport authority (a combination `issue_grant` itself can never
/// produce, but persisted state must still fail closed on if it were ever
/// corrupted into this shape) fails integrity on reload.
#[test]
fn persisted_grant_with_contradictory_endpoint_and_diot_authority_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record.grant.endpoint_id = Some("dcall-endpoint:fog-01:http".to_string());
    record.grant.endpoint_revision = Some(1);
    record.grant.endpoint_descriptor_digest = Some(format!("sha256:{}", "9".repeat(64)));
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("mutually exclusive"),
        "unexpected error: {error}"
    );
}

/// 28E: a persisted grant whose typed authority carries a tampered
/// (shape-invalid) `descriptor_digest` fails integrity on reload.
#[test]
fn persisted_grant_with_tampered_binding_descriptor_digest_fails_closed_on_reload() {
    let db_path = tempfile_db();
    let mut grants = load_state(&db_path).unwrap();
    let fixture = build_v2_fixture("fog-01", "fog-01");
    let route = derive(&fixture, "fog-01").unwrap();
    let mut diot_bindings = DIoTRuntimeBindingRegistryStateV1::empty();
    let authority = diot_transport_authority_for(&fixture, &route, &mut diot_bindings);
    let issued = issue_grant(
        &mut grants,
        route,
        DCallDeliveryAuthorityV1::DiotPubsub(authority),
        "m1".to_string(),
        "c1".to_string(),
        "d".repeat(64),
        4,
        "e".repeat(64),
        now(),
    )
    .unwrap();
    let record = grants.grants.get_mut(&issued.grant.grant_id).unwrap();
    record
        .grant
        .v2_dcall_transport_authority
        .as_mut()
        .unwrap()
        .descriptor_digest = "not-a-real-digest".to_string();
    persist_state(&db_path, &grants).unwrap();
    let error = load_state(&db_path).unwrap_err();
    assert!(
        error.contains("descriptor_digest"),
        "unexpected error: {error}"
    );
}
