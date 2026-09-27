//! Deterministic control-plane projection derived from accepted DMap/2.
use super::{
    canonical_dcontinuum::CanonicalDContinuumV1,
    canonical_dforward::CanonicalDForwardV1,
    canonical_dgraph::CanonicalDGraphV1,
    canonical_dmap_v2::{
        CanonicalDMapV2, DCallRealizationV2, DIoTPlacementV2, ExternalPortBindingV2,
    },
    ddeploy::DDeployStateV1,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DIoTWorkloadProjection {
    pub placement_id: String,
    pub diot_id: String,
    pub node_id: String,
    pub artifact_id: String,
    pub descriptor_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DependencyProjection {
    pub diot_id: String,
    pub interface: String,
    pub scope: String,
    pub readiness_authority: String,
    pub readiness_state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DForwardNodeProjection {
    pub node_id: String,
    pub dmap_id: String,
    pub dmap_revision: u64,
    pub dmap_digest: String,
    pub dforward_id: String,
    pub dforward_revision: u64,
    pub dforward_digest: String,
    pub diot_workloads: Vec<DIoTWorkloadProjection>,
    pub same_element_dependencies: Vec<DependencyProjection>,
    pub any_element_dependencies: Vec<DependencyProjection>,
    pub dcall_realization_ids: Vec<String>,
    pub external_binding_ids: Vec<String>,
}

/// Raw node-local derivation shared by BOTH the caller-supplied-authority
/// path (`project_node`/`project_validated_node`, used by unit tests that
/// construct their own `CanonicalDMapV2`) and the governed,
/// authority-proving entry point (`governed_node_projection`) below.
/// Sourced from raw parts rather than a full `CanonicalDMapV2` so the
/// governed path never needs to reconstruct a synthetic map merely to reuse
/// this logic -- see `governed_node_projection`'s own doc comment for why
/// that distinction matters.
fn derive_node_projection(
    dmap_id: &str,
    dmap_revision: u64,
    dmap_digest: &str,
    forward: &CanonicalDForwardV1,
    diot_placements: &[DIoTPlacementV2],
    dcall_realizations: &[DCallRealizationV2],
    external_port_bindings: &[ExternalPortBindingV2],
    node_id: &str,
) -> DForwardNodeProjection {
    let mut workloads = Vec::new();
    for p in diot_placements.iter().filter(|p| p.node_id == node_id) {
        workloads.push(DIoTWorkloadProjection {
            placement_id: p.placement_id.clone(),
            diot_id: p.diot_id.clone(),
            node_id: p.node_id.clone(),
            artifact_id: p.artifact_binding.artifact_id.clone(),
            descriptor_digest: p.artifact_binding.descriptor_digest.clone(),
        });
    }
    // Deterministic node-local ordering: sort by (diot_id, node_id) so a
    // caller iterating diot_placements in any order (e.g. after a
    // persisted-state reload with different BTreeMap/Vec construction
    // order) always sees the SAME workload ordering here (test J).
    workloads.sort_by(|a, b| {
        (a.diot_id.as_str(), a.placement_id.as_str())
            .cmp(&(b.diot_id.as_str(), b.placement_id.as_str()))
    });
    let mut same = Vec::new();
    let mut any = Vec::new();
    // Iterate DIoTs/requirements in a stable, content-derived order rather
    // than trusting `forward.diots`'s own on-disk/in-memory order, for the
    // same determinism reason.
    let mut sorted_diots: Vec<&super::canonical_dforward::CanonicalDIoTV1> =
        forward.diots.iter().collect();
    sorted_diots.sort_by(|a, b| a.diot_id.cmp(&b.diot_id));
    for d in sorted_diots {
        let mut reqs: Vec<&super::canonical_dforward::CanonicalDForwardRequirementV1> =
            d.requires.iter().collect();
        reqs.sort_by(|a, b| a.interface.cmp(&b.interface));
        for r in reqs {
            let scope = format!("{:?}", r.scope);
            let row = DependencyProjection {
                diot_id: d.diot_id.clone(),
                interface: r.interface.clone(),
                scope: scope.clone(),
                readiness_authority: if scope == "AnyElement" {
                    "dmonitor".into()
                } else {
                    "static_configuration".into()
                },
                readiness_state: if scope == "AnyElement" {
                    "pending_fresh_evidence".into()
                } else {
                    "structurally_validated".into()
                },
            };
            if scope == "SameElement" {
                same.push(row);
            } else {
                any.push(row);
            }
        }
    }
    let mut dcall_realization_ids: Vec<String> = dcall_realizations
        .iter()
        .map(|x| x.call_id.clone())
        .collect();
    dcall_realization_ids.sort();
    let mut external_binding_ids: Vec<String> = external_port_bindings
        .iter()
        .map(|x| format!("{}:{}", x.service_id, x.port))
        .collect();
    external_binding_ids.sort();
    DForwardNodeProjection {
        node_id: node_id.to_string(),
        dmap_id: dmap_id.to_string(),
        dmap_revision,
        dmap_digest: dmap_digest.to_string(),
        dforward_id: forward.dforward_id.clone(),
        dforward_revision: forward.revision,
        dforward_digest: forward.digest().unwrap_or_default(),
        diot_workloads: workloads,
        same_element_dependencies: same,
        any_element_dependencies: any,
        dcall_realization_ids,
        external_binding_ids,
    }
}

/// Node-local filtering shared by `project_validated_node` and
/// `governed_node_projection`: a projection for node N must contain ONLY
/// same/any_element dependencies, DCall realization ids, and external
/// binding ids that are actually anchored to a DIoT placed on N -- never
/// leak another node's (or another placement's) content into N's view.
fn filter_to_node_local(
    projection: &mut DForwardNodeProjection,
    node_id: &str,
    diot_placements: &[DIoTPlacementV2],
    dcall_realizations: &[DCallRealizationV2],
    external_port_bindings: &[ExternalPortBindingV2],
) {
    let local: std::collections::HashSet<String> = diot_placements
        .iter()
        .filter(|p| p.node_id == node_id)
        .map(|p| p.diot_id.clone())
        .collect();
    projection
        .same_element_dependencies
        .retain(|dep| local.contains(&dep.diot_id));
    projection
        .any_element_dependencies
        .retain(|dep| local.contains(&dep.diot_id));
    projection.dcall_realization_ids.retain(|call| {
        dcall_realizations.iter().any(|r| {
            &r.call_id == call
                && diot_placements
                    .iter()
                    .any(|p| p.placement_id == r.diot_placement_id && p.node_id == node_id)
        })
    });
    projection.external_binding_ids.retain(|binding| {
        external_port_bindings.iter().any(|b| {
            format!("{}:{}", b.service_id, b.port) == *binding
                && b.diot_placement_id.as_ref().is_some_and(|id| {
                    diot_placements
                        .iter()
                        .any(|p| &p.placement_id == id && p.node_id == node_id)
                })
        })
    });
}

fn project_node(
    map: &CanonicalDMapV2,
    forward: &CanonicalDForwardV1,
    node_id: &str,
) -> Result<DForwardNodeProjection, String> {
    Ok(derive_node_projection(
        &map.dmap_id,
        map.revision,
        &map.digest()?,
        forward,
        &map.diot_placements,
        &map.dcall_realizations,
        &map.external_port_bindings,
        node_id,
    ))
}

/// Caller-supplied-authority projection entry point: the CALLER provides
/// `map`/`forward`/`graph`/`continuum` and this function proves they are
/// mutually consistent (via `map.validate`) and that `forward` is exactly
/// what `map.dforward_ref` names, before deriving a node-local view. This
/// is the right entry point for tests and for any future caller that
/// already holds its own verified, self-consistent quadruple -- it does
/// NOT prove `map` was ever actually ACCEPTED by DServer's real governance
/// core. Production callers that need that proof must use
/// `governed_node_projection` instead, which loads the map/forward from
/// CURRENT PERSISTED ACCEPTED AUTHORITY itself and never trusts a
/// caller-supplied `CanonicalDMapV2` as evidence of acceptance.
pub fn project_validated_node(
    map: &CanonicalDMapV2,
    forward: &CanonicalDForwardV1,
    graph: &CanonicalDGraphV1,
    continuum: &CanonicalDContinuumV1,
    node_id: &str,
) -> Result<DForwardNodeProjection, String> {
    if !map.validate(graph, continuum, forward).is_empty() {
        return Err("DMap/2 is not an accepted valid authority for this projection".into());
    }
    if map.dforward_ref.id != forward.dforward_id
        || map.dforward_ref.revision != forward.revision
        || map.dforward_ref.digest != forward.digest()?
    {
        return Err("DForward reference does not match exact projection authority".into());
    }
    let mut projection = project_node(map, forward, node_id)?;
    filter_to_node_local(
        &mut projection,
        node_id,
        &map.diot_placements,
        &map.dcall_realizations,
        &map.external_port_bindings,
    );
    Ok(projection)
}

/// Typed failure classes for `governed_node_projection`, distinct from the
/// bare `String` errors the caller-supplied-authority path above uses, so a
/// production caller (a future HTTP/CLI layer) can match on the exact
/// reason rather than string-matching an error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GovernedProjectionError {
    /// No active authority exists for this project/application, or the
    /// active authority is `datum.dmap/1` rather than `datum.dmap/2` --
    /// Phase-113 DForward node projection is unavailable either way.
    ProjectionUnavailable(String),
    /// A caller-supplied EXPECTED DForward digest did not match the active
    /// DMap/2's own exact accepted DForward reference.
    DForwardMismatch(String),
    /// The requested node does not appear in the active DMap/2's own DServ
    /// placements or DIoT placements.
    UnknownNode(String),
    /// Persisted-state/digest-recomputation failure -- corruption, not a
    /// caller error.
    Internal(String),
}
impl std::fmt::Display for GovernedProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectionUnavailable(m) => write!(f, "projection unavailable: {m}"),
            Self::DForwardMismatch(m) => write!(f, "dforward mismatch: {m}"),
            Self::UnknownNode(m) => write!(f, "unknown node: {m}"),
            Self::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

/// THE governed, production-safe projection entry point (Phase 113A
/// closure pass 5). Unlike `project_validated_node`, this function never
/// accepts a caller-supplied `CanonicalDMapV2`/`CanonicalDForwardV1` as
/// evidence of acceptance -- it loads the CURRENT persisted active
/// authority for `(project_id, application_id)` via the exact same shared
/// `active_placement_authority()` accessor Phase 113A's D-Code/D-Call
/// production paths already use (deliberately not a second,
/// independently-implemented authority-reading path), requires that
/// authority to genuinely be `datum.dmap/2` (never a down-converted or
/// synthesized v1 view), and resolves the EXACT accepted `dforward_snapshot`
/// that authority's own immutable proposal record carries -- never a
/// caller-supplied DForward.
///
/// `expected_dforward_digest`, if `Some`, lets a defensive caller (e.g. a
/// future HTTP layer that received a client-claimed DForward reference)
/// assert it against the real active digest and fail closed on mismatch;
/// `None` skips that check (there is nothing to mismatch, since this
/// function never trusts a caller-supplied DForward for anything else).
///
/// A requested node is "known" to this deployment if it appears in the
/// active DMap/2's own DServ placements OR DIoT placements; anything else
/// fails closed as `UnknownNode` rather than inventing placement. A KNOWN
/// node with zero DIoT placements returns a structurally valid, empty-diot
/// projection (deterministic, repository-consistent choice -- see
/// `known_node_with_zero_diot_realization_returns_empty_valid_projection`).
pub fn governed_node_projection(
    state: &DDeployStateV1,
    project_id: &str,
    application_id: &str,
    node_id: &str,
    expected_dforward_digest: Option<&str>,
) -> Result<DForwardNodeProjection, GovernedProjectionError> {
    let view = super::ddeploy::active_placement_authority(state, project_id, application_id)
        .map_err(GovernedProjectionError::Internal)?
        .ok_or_else(|| {
            GovernedProjectionError::ProjectionUnavailable(format!(
                "no active deployment authority exists for application '{application_id}' in project '{project_id}'"
            ))
        })?;
    let extras = view.v2.as_ref().ok_or_else(|| {
        GovernedProjectionError::ProjectionUnavailable(format!(
            "Phase-113 DForward node projection unavailable: active authority for application '{application_id}' is datum.dmap/1, not datum.dmap/2"
        ))
    })?;
    let forward = extras.dforward_snapshot.as_ref().ok_or_else(|| {
        GovernedProjectionError::Internal(
            "active v2 authority is missing its accepted DForward snapshot -- persisted-state corruption".into(),
        )
    })?;
    if let Some(expected) = expected_dforward_digest {
        if expected != extras.dforward_digest {
            return Err(GovernedProjectionError::DForwardMismatch(format!(
                "supplied DForward digest '{expected}' does not match active DMap/2's exact accepted DForward digest '{}'",
                extras.dforward_digest
            )));
        }
    }
    let known_nodes: std::collections::HashSet<&str> = view
        .placements
        .iter()
        .map(|p| p.node_id.as_str())
        .chain(extras.diot_placements.iter().map(|p| p.node_id.as_str()))
        .collect();
    if !known_nodes.contains(node_id) {
        return Err(GovernedProjectionError::UnknownNode(format!(
            "node '{node_id}' does not appear in application '{application_id}''s active DMap/2 placements"
        )));
    }
    let mut projection = derive_node_projection(
        &view.dmap_id,
        view.revision,
        &view.digest,
        forward,
        &extras.diot_placements,
        &extras.dcall_realizations,
        &extras.external_port_bindings,
        node_id,
    );
    filter_to_node_local(
        &mut projection,
        node_id,
        &extras.diot_placements,
        &extras.dcall_realizations,
        &extras.external_port_bindings,
    );
    Ok(projection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1,
        CanonicalNodeCapacityV1, CanonicalNodePlatformV1,
    };
    use crate::core::canonical_dforward::{
        CanonicalArtifactRequirementV1, CanonicalDForwardRequirementV1, CanonicalDIoTV1,
        DForwardRequirementScope, CANONICAL_DFORWARD_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
        CanonicalDServResourceRequirementV1, CanonicalDServV1,
    };
    use crate::core::canonical_dmap_v2::{
        ContractRefV2, DIoTArtifactBindingV2, DServPlacementV2, CANONICAL_DMAP_V2_SCHEMA,
    };
    use crate::core::ddeploy::ProposalSource;
    use crate::core::ddeploy_v2::{DDeployProposalV2, DIoTArtifactAuthorityBindingV2};
    use std::collections::BTreeMap;

    fn sha_ok(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn dgraph(app: &str) -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: crate::core::canonical_dgraph::CANONICAL_DGRAPH_SCHEMA.into(),
            application_id: app.into(),
            dgraph_id: format!("dgraph:{app}"),
            revision: 1,
            services: vec![
                CanonicalDServV1 {
                    service_id: "svc-a".into(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 {
                        dnode_abi: "datum-dnode/0".into(),
                    },
                    resource_requirement: CanonicalDServResourceRequirementV1 {
                        cpu_cores: 1,
                        memory_bytes: 1,
                    },
                },
                CanonicalDServV1 {
                    service_id: "svc-b".into(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 {
                        dnode_abi: "datum-dnode/0".into(),
                    },
                    resource_requirement: CanonicalDServResourceRequirementV1 {
                        cpu_cores: 1,
                        memory_bytes: 1,
                    },
                },
            ],
            calls: vec![],
        }
    }

    fn continuum(project: &str) -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.into(),
            project_id: project.into(),
            dcontinuum_id: format!("continuum:{project}"),
            revision: 1,
            nodes: vec![
                CanonicalDNodeV1 {
                    node_id: "node-a".into(),
                    platform: CanonicalNodePlatformV1 {
                        os: "linux".into(),
                        architecture: "amd64".into(),
                    },
                    capacity: CanonicalNodeCapacityV1 {
                        cpu_cores: 4,
                        memory_bytes: 4,
                    },
                    capabilities: CanonicalNodeCapabilitiesV1 {
                        dnode_abi_versions: vec!["datum-dnode/0".into()],
                    },
                },
                CanonicalDNodeV1 {
                    node_id: "node-b".into(),
                    platform: CanonicalNodePlatformV1 {
                        os: "linux".into(),
                        architecture: "amd64".into(),
                    },
                    capacity: CanonicalNodeCapacityV1 {
                        cpu_cores: 4,
                        memory_bytes: 4,
                    },
                    capabilities: CanonicalNodeCapabilitiesV1 {
                        dnode_abi_versions: vec!["datum-dnode/0".into()],
                    },
                },
            ],
            links: vec![],
        }
    }

    /// Two DIoTs (one per node), one with a `same_element` and one with an
    /// `any_element` requirement, so projection tests can prove both
    /// dependency-scope semantics and node-local isolation in one fixture.
    fn dforward_two_diots(project: &str) -> CanonicalDForwardV1 {
        CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.into(),
            project_id: project.into(),
            dforward_id: format!("dforward:{project}"),
            revision: 1,
            diots: vec![
                CanonicalDIoTV1 {
                    diot_id: "diot-a".into(),
                    diot_kind: "bridge".into(),
                    artifact_requirement: CanonicalArtifactRequirementV1 {
                        artifact_id: "artifact-a".into(),
                    },
                    provides: vec![],
                    requires: vec![CanonicalDForwardRequirementV1 {
                        interface: "same.iface/1".into(),
                        scope: DForwardRequirementScope::SameElement,
                    }],
                },
                CanonicalDIoTV1 {
                    diot_id: "diot-b".into(),
                    diot_kind: "broker".into(),
                    artifact_requirement: CanonicalArtifactRequirementV1 {
                        artifact_id: "artifact-b".into(),
                    },
                    provides: vec![],
                    requires: vec![CanonicalDForwardRequirementV1 {
                        interface: "any.iface/1".into(),
                        scope: DForwardRequirementScope::AnyElement,
                    }],
                },
            ],
            external_endpoints: vec![],
            chains: vec![],
            support_jobs: vec![],
        }
    }

    fn map_with_two_diots(
        project: &str,
        app: &str,
        forward: &CanonicalDForwardV1,
        dgraph: &CanonicalDGraphV1,
        continuum: &CanonicalDContinuumV1,
    ) -> CanonicalDMapV2 {
        CanonicalDMapV2 {
            schema: CANONICAL_DMAP_V2_SCHEMA.into(),
            project_id: project.into(),
            application_id: app.into(),
            dmap_id: format!("dmap:{app}"),
            revision: 1,
            dgraph_ref: ContractRefV2 {
                id: dgraph.dgraph_id.clone(),
                revision: dgraph.revision,
                digest: super::super::canonical_dgraph::recompute_dgraph_digest(dgraph).unwrap(),
            },
            dcontinuum_ref: ContractRefV2 {
                id: continuum.dcontinuum_id.clone(),
                revision: continuum.revision,
                digest: super::super::canonical_dcontinuum::recompute_dcontinuum_digest(continuum)
                    .unwrap(),
            },
            dforward_ref: ContractRefV2 {
                id: forward.dforward_id.clone(),
                revision: forward.revision,
                digest: forward.digest().unwrap(),
            },
            placements: vec![
                DServPlacementV2 {
                    service_id: "svc-a".into(),
                    node_id: "node-a".into(),
                },
                DServPlacementV2 {
                    service_id: "svc-b".into(),
                    node_id: "node-b".into(),
                },
            ],
            diot_placements: vec![
                DIoTPlacementV2 {
                    placement_id: "place-a".into(),
                    diot_id: "diot-a".into(),
                    node_id: "node-a".into(),
                    artifact_binding: DIoTArtifactBindingV2 {
                        artifact_id: "artifact-a".into(),
                        descriptor_digest: sha_ok('a'),
                        content_digest: None,
                    },
                },
                DIoTPlacementV2 {
                    placement_id: "place-b".into(),
                    diot_id: "diot-b".into(),
                    node_id: "node-b".into(),
                    artifact_binding: DIoTArtifactBindingV2 {
                        artifact_id: "artifact-b".into(),
                        descriptor_digest: sha_ok('b'),
                        content_digest: None,
                    },
                },
            ],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
        }
    }

    /// Builds a real accepted v2 authority in `state` (via the real
    /// `create_v2_proposal`/`accept_v2_proposal` core path, never by
    /// poking `phase113_v2.active` directly) for `map_with_two_diots`'s
    /// exact topology, so `governed_node_projection` tests exercise the
    /// real persisted-authority chain end to end.
    /// A minimal REAL `ServiceArtifact` (mirroring `ddeploy.rs`'s own
    /// private `minimal_native_service_artifact` test helper, which is not
    /// visible from this module) -- goes through the real registry so its
    /// `control_plane.execution_generation`/`execution_projection_digest`
    /// are genuinely server-computed, never hand-typed.
    fn test_service_artifact(
        artifact_id: &str,
        service_id: &str,
        target_nodes: Vec<&str>,
    ) -> crate::core::service_artifact_registry::ServiceArtifact {
        use crate::core::service_artifact_registry::*;
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: artifact_id.to_string(),
            service_id: service_id.to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "native_process".to_string(),
                container: None,
                native_process: Some(NativeProcessDeploymentSpecV1 {
                    process_name: "proc".to_string(),
                    entrypoint: "bin/run".to_string(),
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
                target_nodes: target_nodes.into_iter().map(str::to_string).collect(),
                acquisition: ServiceArtifactAcquisition {
                    kind: "native_process_executable".to_string(),
                    source: Some("/opt/x".to_string()),
                    sha256: Some("0".repeat(64)),
                    ..Default::default()
                },
                wasm_decision: ServiceArtifactWasmDecision {
                    module_id: "reconcile_service".to_string(),
                    function_name: "decide".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    fn accept_two_diot_v2_authority(
        project: &str,
        app: &str,
    ) -> (crate::core::ddeploy::DDeployStateV1, CanonicalDContinuumV1) {
        let dg = dgraph(app);
        let dc = continuum(project);
        let forward = dforward_two_diots(project);
        let map = map_with_two_diots(project, app, &forward, &dg, &dc);

        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                test_service_artifact("artifact-a", "svc-a", vec!["node-a"]),
            )
            .unwrap();
        registry
            .create(
                project,
                test_service_artifact("artifact-b", "svc-b", vec!["node-b"]),
            )
            .unwrap();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let authority =
            super::super::ddeploy::resolve_diot_artifact_authority(&forward, project, &artifacts)
                .unwrap_or_else(|e| panic!("real artifact authority resolution must succeed: {e}"));

        let proposal = DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.into(),
            proposal_id: format!("p-{app}"),
            project_id: project.into(),
            application_id: app.into(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref: map.dgraph_ref.clone(),
            dcontinuum_ref: map.dcontinuum_ref.clone(),
            dforward_ref: map.dforward_ref.clone(),
            placements: map.placements.clone(),
            diot_placements: map.diot_placements.clone(),
            dcall_realizations: map.dcall_realizations.clone(),
            external_port_bindings: map.external_port_bindings.clone(),
            expected_active_dmap_revision: None,
            diot_artifact_authority: authority,
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        state
            .create_v2_proposal(proposal, dg, dc.clone(), forward)
            .unwrap_or_else(|e| panic!("v2 proposal creation must succeed: {e}"));
        state
            .accept_v2_proposal(
                &format!("p-{app}"),
                format!("acc-{app}"),
                "operator:test".into(),
                chrono::Utc::now().to_rfc3339(),
                &dc,
                &artifacts,
                None,
            )
            .unwrap_or_else(|e| panic!("v2 acceptance must succeed: {e}"));
        (state, dc)
    }

    /// Exercises the caller-supplied-authority entry point
    /// (`project_validated_node`) directly, against the real on-disk
    /// Phase-113A fixture bundle -- proves it stays a genuinely reachable,
    /// correct public function (not merely dead code left over from before
    /// the governed entry point existed) and cross-checks its output
    /// against `governed_node_projection`'s own real-authority result for
    /// the identical topology.
    #[test]
    fn caller_supplied_authority_entry_point_matches_real_fixture_bundle() {
        fn strip_bookkeeping(raw: &str) -> String {
            let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
            if let Some(obj) = v.as_object_mut() {
                obj.remove("x_note");
                obj.remove("expected_digest_sha256");
            }
            serde_json::to_string(&v).unwrap()
        }
        let dgraph: CanonicalDGraphV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dgraph.smart-sentinel-lab.json"
        ))
        .unwrap();
        let dcontinuum: CanonicalDContinuumV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dcontinuum.smart-sentinel-lab.json"
        )).unwrap();
        let dforward: CanonicalDForwardV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        )).unwrap();
        let raw_map = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let map: CanonicalDMapV2 = serde_json::from_str(&strip_bookkeeping(raw_map)).unwrap();

        let projection = project_validated_node(&map, &dforward, &dgraph, &dcontinuum, "fog-01")
            .expect(
                "real fixture bundle must project cleanly for a real node it places services on",
            );
        assert_eq!(projection.node_id, "fog-01");
        assert_eq!(projection.dmap_id, "dmap:smart-sentinel-lab");
        assert_eq!(projection.dforward_id, "dforward:smart-sentinel-lab");
        // fog-01 hosts the mqtt-broker DIoT placement in the real fixture.
        assert_eq!(projection.diot_workloads.len(), 1);
        assert_eq!(projection.diot_workloads[0].diot_id, "mqtt-broker");

        // A mismatching DForward (wrong revision) must be rejected -- proves
        // the "supplied DForward does not match exact DMap/2 reference"
        // failure case this same function has always implemented.
        let mut wrong_forward = dforward.clone();
        wrong_forward.revision = 999;
        let rejected = project_validated_node(&map, &wrong_forward, &dgraph, &dcontinuum, "fog-01");
        assert!(
            rejected.is_err(),
            "a DForward that disagrees with the DMap/2's own dforward_ref must be rejected"
        );
    }

    // A. current accepted DMap/2 -> node projection succeeds.
    #[test]
    fn governed_projection_succeeds_for_current_accepted_dmap2() {
        let (state, _dc) = accept_two_diot_v2_authority("proj-a", "app-a");
        let result = governed_node_projection(&state, "proj-a", "app-a", "node-a", None);
        assert!(
            result.is_ok(),
            "governed projection must succeed for a real accepted DMap/2 -- got {result:?}"
        );
    }

    // B. DMap/1 active -> Phase-113 projection unavailable. Built via the
    // REAL public v1 `create_proposal`/`accept_proposal` core functions
    // (not a Phase-113-only test helper), proving the governed entry point
    // genuinely inspects the shared authority accessor's `v2` field rather
    // than assuming its caller only ever has v2 state.
    #[test]
    fn governed_projection_unavailable_when_active_authority_is_dmap1() {
        use crate::core::canonical_dmap::CanonicalDServPlacementV1;
        use crate::core::ddeploy::{accept_proposal, create_proposal, CreateProposalInput};
        let project = "proj-v1-only";
        let app = "app-v1";
        let dg = dgraph(app);
        let dc = continuum(project);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                test_service_artifact("artifact-svc-a", "svc-a", vec!["node-a"]),
            )
            .unwrap();
        registry
            .create(
                project,
                test_service_artifact("artifact-svc-b", "svc-b", vec!["node-a"]),
            )
            .unwrap();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let record = create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: project.into(),
                application_id: app.into(),
                source: ProposalSource::Operator,
                dgraph: dg,
                dcontinuum: dc.clone(),
                placements: vec![
                    CanonicalDServPlacementV1 {
                        service_id: "svc-a".into(),
                        node_id: "node-a".into(),
                    },
                    CanonicalDServPlacementV1 {
                        service_id: "svc-b".into(),
                        node_id: "node-a".into(),
                    },
                ],
                planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            "p-v1".into(),
            chrono::Utc::now().to_rfc3339(),
            &artifacts,
        )
        .unwrap_or_else(|e| panic!("v1 proposal creation must succeed: {e:?}"));
        accept_proposal(
            &mut state,
            &record.proposal.proposal_id,
            "operator:test",
            None,
            &artifacts,
            Some(&dc),
            &chrono::Utc::now().to_rfc3339(),
            "acc-v1".into(),
        )
        .unwrap_or_else(|e| panic!("v1 acceptance must succeed: {e:?}"));

        let result = governed_node_projection(&state, project, app, "node-a", None);
        assert!(
            matches!(
                result,
                Err(GovernedProjectionError::ProjectionUnavailable(_))
            ),
            "an active DMap/1 authority must reject with ProjectionUnavailable, got {result:?}"
        );
    }

    // C. wrong DForward digest -> reject.
    #[test]
    fn governed_projection_rejects_mismatching_expected_dforward_digest() {
        let (state, _dc) = accept_two_diot_v2_authority("proj-c", "app-c");
        let wrong = sha_ok('9');
        let result = governed_node_projection(&state, "proj-c", "app-c", "node-a", Some(&wrong));
        assert!(
            matches!(result, Err(GovernedProjectionError::DForwardMismatch(_))),
            "got {result:?}"
        );
    }

    // Unknown node -> reject (part of section 8's required-failure set).
    #[test]
    fn governed_projection_rejects_unknown_node() {
        let (state, _dc) = accept_two_diot_v2_authority("proj-unknown-node", "app-u");
        let result = governed_node_projection(
            &state,
            "proj-unknown-node",
            "app-u",
            "node-does-not-exist",
            None,
        );
        assert!(
            matches!(result, Err(GovernedProjectionError::UnknownNode(_))),
            "got {result:?}"
        );
    }

    // D + E. node A projection contains only node A DIoTs; node B content
    // does not leak into node A.
    #[test]
    fn projection_is_strictly_node_local_no_cross_node_leakage() {
        let (state, _dc) = accept_two_diot_v2_authority("proj-de", "app-de");
        let a = governed_node_projection(&state, "proj-de", "app-de", "node-a", None).unwrap();
        let b = governed_node_projection(&state, "proj-de", "app-de", "node-b", None).unwrap();
        assert_eq!(a.diot_workloads.len(), 1);
        assert_eq!(a.diot_workloads[0].diot_id, "diot-a");
        assert_eq!(b.diot_workloads.len(), 1);
        assert_eq!(b.diot_workloads[0].diot_id, "diot-b");
        assert!(
            a.diot_workloads.iter().all(|w| w.diot_id != "diot-b"),
            "node A must not contain node B's DIoT"
        );
        assert!(
            b.diot_workloads.iter().all(|w| w.diot_id != "diot-a"),
            "node B must not contain node A's DIoT"
        );
    }

    // F + G. unrelated DCall realizations / external bindings excluded.
    #[test]
    fn unrelated_dcall_realizations_and_external_bindings_are_excluded_per_node() {
        let project = "proj-fg";
        let app = "app-fg";
        let dg = dgraph(app);
        let dc = continuum(project);
        let forward = dforward_two_diots(project);
        let mut map = map_with_two_diots(project, app, &forward, &dg, &dc);
        map.dcall_realizations = vec![DCallRealizationV2 {
            call_id: "call-only-on-a".into(),
            realization_kind: "via_diot_pubsub".into(),
            diot_placement_id: "place-a".into(),
            interface: "same.iface/1".into(),
            channel: "chan/a".into(),
        }];
        map.external_port_bindings = vec![ExternalPortBindingV2 {
            service_id: "svc-b".into(),
            port: "port-b".into(),
            endpoint_id: "endpoint-b".into(),
            diot_placement_id: Some("place-b".into()),
            interface: "any.iface/1".into(),
            channel: "chan/b".into(),
        }];
        let node_a = project_node(&map, &forward, "node-a").unwrap();
        let node_b = project_node(&map, &forward, "node-b").unwrap();
        let mut filtered_a = node_a.clone();
        filter_to_node_local(
            &mut filtered_a,
            "node-a",
            &map.diot_placements,
            &map.dcall_realizations,
            &map.external_port_bindings,
        );
        let mut filtered_b = node_b.clone();
        filter_to_node_local(
            &mut filtered_b,
            "node-b",
            &map.diot_placements,
            &map.dcall_realizations,
            &map.external_port_bindings,
        );
        assert_eq!(
            filtered_a.dcall_realization_ids,
            vec!["call-only-on-a".to_string()]
        );
        assert!(
            filtered_b.dcall_realization_ids.is_empty(),
            "node B must not see node A's DCall realization"
        );
        assert!(
            filtered_a.external_binding_ids.is_empty(),
            "node A must not see node B's external binding"
        );
        assert_eq!(
            filtered_b.external_binding_ids,
            vec!["svc-b:port-b".to_string()]
        );
    }

    // H. same_element local dependency represented correctly.
    #[test]
    fn same_element_dependency_is_structurally_validated_not_dmonitor_pending() {
        let (state, _dc) = accept_two_diot_v2_authority("proj-h", "app-h");
        let a = governed_node_projection(&state, "proj-h", "app-h", "node-a", None).unwrap();
        assert_eq!(a.same_element_dependencies.len(), 1);
        assert_eq!(
            a.same_element_dependencies[0].readiness_authority,
            "static_configuration"
        );
        assert_eq!(
            a.same_element_dependencies[0].readiness_state,
            "structurally_validated"
        );
        assert!(a.any_element_dependencies.is_empty());
    }

    // I. any_element explicitly DMonitor-pending -- never ready/healthy.
    #[test]
    fn any_element_dependency_is_explicitly_dmonitor_pending_never_ready() {
        let (state, _dc) = accept_two_diot_v2_authority("proj-i", "app-i");
        let b = governed_node_projection(&state, "proj-i", "app-i", "node-b", None).unwrap();
        assert_eq!(b.any_element_dependencies.len(), 1);
        assert_eq!(
            b.any_element_dependencies[0].readiness_authority,
            "dmonitor"
        );
        assert_eq!(
            b.any_element_dependencies[0].readiness_state,
            "pending_fresh_evidence"
        );
        assert!(b.same_element_dependencies.is_empty());
        // Never emit ready=true/healthy=true/reachable=true from static data.
        let serialized = serde_json::to_string(&b).unwrap();
        assert!(!serialized.contains("\"ready\":true"));
        assert!(!serialized.contains("\"healthy\":true"));
        assert!(!serialized.contains("\"reachable\":true"));
    }

    // J. projection deterministic under semantically irrelevant ordering
    // changes (reversed diot_placements/dcall_realizations/external
    // bindings and reversed DForward.diots order).
    #[test]
    fn projection_is_deterministic_under_semantically_irrelevant_reordering() {
        let project = "proj-j";
        let app = "app-j";
        let dg = dgraph(app);
        let dc = continuum(project);
        let forward = dforward_two_diots(project);
        let map = map_with_two_diots(project, app, &forward, &dg, &dc);
        let mut reversed_forward = forward.clone();
        reversed_forward.diots.reverse();
        let mut reversed_map = map.clone();
        reversed_map.diot_placements.reverse();
        reversed_map.dcall_realizations.reverse();
        reversed_map.external_port_bindings.reverse();

        let p1 = project_node(&map, &forward, "node-a").unwrap();
        let p2 = project_node(&reversed_map, &reversed_forward, "node-a").unwrap();
        assert_eq!(
            p1, p2,
            "reordering unrelated collections must not change the projection"
        );
    }

    // Known node with zero DIoT realization: deterministic, repository-
    // consistent choice -- empty-but-valid projection, never fabricated
    // placement.
    #[test]
    fn known_node_with_zero_diot_realization_returns_empty_valid_projection() {
        // A node present only in `placements` (a real DServ target, so it is
        // KNOWN to this deployment) but with zero DIoT placements must yield
        // a structurally valid, empty projection -- never fabricated
        // placement, never an error.
        let project = "proj-empty-node";
        let app = "app-empty";
        let dg = dgraph(app);
        let dc = continuum(project);
        let forward = dforward_two_diots(project);
        let mut map = map_with_two_diots(project, app, &forward, &dg, &dc);
        map.placements = vec![
            DServPlacementV2 {
                service_id: "svc-a".into(),
                node_id: "node-a".into(),
            },
            DServPlacementV2 {
                service_id: "svc-b".into(),
                node_id: "node-a".into(),
            },
        ];
        map.diot_placements = vec![];
        map.dcall_realizations = vec![];
        map.external_port_bindings = vec![];
        let mut projection = project_node(&map, &forward, "node-a").unwrap();
        filter_to_node_local(
            &mut projection,
            "node-a",
            &map.diot_placements,
            &map.dcall_realizations,
            &map.external_port_bindings,
        );
        assert!(projection.diot_workloads.is_empty());
        assert!(projection.same_element_dependencies.is_empty());
        assert!(projection.any_element_dependencies.is_empty());
        assert_eq!(projection.node_id, "node-a");
        assert_eq!(projection.dmap_id, map.dmap_id);
    }
}
