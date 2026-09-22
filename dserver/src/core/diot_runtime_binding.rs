//! DIoT runtime-binding registry (Phase 113B1, ADR-0023). Server-owned
//! OPERATIONAL record of which concrete broker transport a governed DIoT
//! placement's D-Call realization actually resolves to. A binding is
//! registered for one `(project_id, application_id, placement_id)` and
//! carries the exact DMap/2 + DForward + artifact-execution-authority
//! identity it was registered under, so a later consumer can detect
//! staleness (`verify_runtime_binding_still_current`) without a second
//! independent lookup.
//!
//! Every identity field except `broker_uri` itself is DERIVED from the
//! currently active DMap/2 placement authority
//! (`crate::core::ddeploy::active_placement_authority`) -- a caller supplies
//! only the placement it is registering for and the broker's URI; it can
//! never declare a `diot_id`/`node_id`/`dmap_ref`/`dforward_ref`/artifact
//! authority of its own. This mirrors `dcall_endpoint`'s structural pattern
//! (mutable-with-monotonic-revision, `descriptor_digest` via
//! `jcs::canonical_digest_sha256_hex`, full re-validation on reload, not
//! merely a digest recompute).
//!
//! This module performs NO MQTT network I/O of any kind: no client,
//! connection, publish, subscribe, retained message, QoS, or credential
//! handling exists here or in any caller. It records binding IDENTITY only,
//! exactly as `dcall_endpoint::DCallEndpointRegistryStateV1` records HTTP
//! routing configuration without itself opening a socket. Real MQTT
//! transport is a later closure.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::ddeploy::DDeployStateV1;
use crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2;
use crate::core::dnode_registry::DNodeRegistryStateV1;
use crate::core::jcs;
use crate::storage::control_state::{
    load_domain_json, persist_domain_json, DOMAIN_DIOT_RUNTIME_BINDING_REGISTRY,
};

pub const DIOT_RUNTIME_BINDING_REGISTRY_STATE_VERSION: &str = "diot_runtime_binding_registry_v1";

/// v0.1's one recognized transport kind -- the only realization_kind DMap/2
/// validation ever accepts is `"via_diot_pubsub"`, and this is the transport
/// that realizes it. Restricting the stored value here (rather than
/// accepting an arbitrary string) mirrors `dcall_endpoint::TRANSPORT_HTTP`'s
/// own rationale: the cheapest way to guarantee an unsupported transport
/// fails closed instead of silently being accepted by a registry nothing
/// implements delivery for yet.
pub const TRANSPORT_MQTT: &str = "mqtt";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

fn length_prefixed_component(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

pub fn binding_key(project_id: &str, application_id: &str, placement_id: &str) -> String {
    format!(
        "{}{}{}",
        length_prefixed_component(project_id),
        length_prefixed_component(application_id),
        length_prefixed_component(placement_id)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DIoTRuntimeBindingV1 {
    pub binding_id: String,
    pub binding_revision: u64,
    pub project_id: String,
    pub application_id: String,
    pub placement_id: String,
    pub diot_id: String,
    pub node_id: String,
    pub dmap_id: String,
    pub dmap_revision: u64,
    pub dmap_digest: String,
    pub dforward_id: String,
    pub dforward_revision: u64,
    pub dforward_digest: String,
    pub artifact_id: String,
    pub execution_generation: u64,
    pub execution_projection_digest: String,
    pub transport: String,
    pub broker_uri: String,
    pub registered_at_utc: String,
    /// Canonical JCS+SHA-256 digest (ADR-0006) of this descriptor's own
    /// content (everything except this field itself).
    pub descriptor_digest: String,
}

impl DIoTRuntimeBindingV1 {
    fn digest_input(&self) -> serde_json::Value {
        serde_json::json!({
            "binding_id": self.binding_id,
            "binding_revision": self.binding_revision,
            "project_id": self.project_id,
            "application_id": self.application_id,
            "placement_id": self.placement_id,
            "diot_id": self.diot_id,
            "node_id": self.node_id,
            "dmap_id": self.dmap_id,
            "dmap_revision": self.dmap_revision,
            "dmap_digest": self.dmap_digest,
            "dforward_id": self.dforward_id,
            "dforward_revision": self.dforward_revision,
            "dforward_digest": self.dforward_digest,
            "artifact_id": self.artifact_id,
            "execution_generation": self.execution_generation,
            "execution_projection_digest": self.execution_projection_digest,
            "transport": self.transport,
            "broker_uri": self.broker_uri,
            "registered_at_utc": self.registered_at_utc,
        })
    }
}

pub fn recompute_binding_digest(binding: &DIoTRuntimeBindingV1) -> Result<String, String> {
    jcs::canonical_digest_sha256_hex(&binding.digest_input()).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DIoTRuntimeBindingRegistryStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub bindings: BTreeMap<String, DIoTRuntimeBindingV1>,
}

impl DIoTRuntimeBindingRegistryStateV1 {
    pub fn empty() -> Self {
        Self {
            state_version: DIOT_RUNTIME_BINDING_REGISTRY_STATE_VERSION.to_string(),
            bindings: BTreeMap::new(),
        }
    }

    pub fn get(
        &self,
        project_id: &str,
        application_id: &str,
        placement_id: &str,
    ) -> Option<&DIoTRuntimeBindingV1> {
        self.bindings
            .get(&binding_key(project_id, application_id, placement_id))
    }
}

/// Validates a `broker_uri` structurally: an explicit `mqtt`/`mqtts`
/// scheme, no embedded userinfo (credentials never belong in a routing
/// URL -- mirrors `dcall_endpoint::validate_base_url`), a parseable host,
/// and no query/fragment component. This is pure string validation -- it
/// never opens a connection.
fn validate_broker_uri(broker_uri: &str) -> Result<(), CanonicalValidationFinding> {
    let parsed = url::Url::parse(broker_uri).map_err(|error| {
        finding(
            "diot_runtime_binding_broker_uri_invalid",
            format!("broker_uri '{broker_uri}' is not a valid URI: {error}"),
        )
    })?;
    if parsed.scheme() != "mqtt" && parsed.scheme() != "mqtts" {
        return Err(finding(
            "diot_runtime_binding_broker_scheme_unsupported",
            format!(
                "broker_uri scheme '{}' is not a supported DIoT broker scheme (only 'mqtt'/'mqtts')",
                parsed.scheme()
            ),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(finding(
            "diot_runtime_binding_broker_uri_invalid",
            format!("broker_uri '{broker_uri}' has no host"),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(finding(
            "diot_runtime_binding_broker_uri_userinfo_rejected",
            "broker_uri must not contain embedded userinfo (credentials never belong in a routing URL)".to_string(),
        ));
    }
    if parsed.query().is_some() {
        return Err(finding(
            "diot_runtime_binding_broker_uri_query_rejected",
            "broker_uri must not contain a query component",
        ));
    }
    if parsed.fragment().is_some() {
        return Err(finding(
            "diot_runtime_binding_broker_uri_fragment_rejected",
            "broker_uri must not contain a fragment component",
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub enum RegisterDIoTRuntimeBindingError {
    Invalid(CanonicalValidationFinding),
    NoActiveDMap2Authority(String),
    PlacementUnknown(String),
    NodeUnknown(String),
    ArtifactAuthorityMissing(String),
}

/// Resolves the `diot_artifact_authority` map an accepted DMap/2 proposal
/// recorded at acceptance time, for the CURRENTLY active application
/// authority. Returns `None` only if internal state is inconsistent (an
/// active authority whose own proposal record is missing) -- unreachable
/// through the normal `create_v2_proposal`/`accept_v2_proposal` path, but
/// surfaced as `None` rather than panicking.
fn current_diot_artifact_authority<'a>(
    ddeploy_state: &'a DDeployStateV1,
    project_id: &str,
    application_id: &str,
) -> Option<&'a BTreeMap<String, DIoTArtifactAuthorityBindingV2>> {
    let key = crate::core::ddeploy::application_key(project_id, application_id);
    let active = ddeploy_state.phase113_v2.active.get(&key)?;
    let record = ddeploy_state.phase113_v2.proposals.get(&active.proposal_id)?;
    Some(&record.proposal.diot_artifact_authority)
}

/// Registers (or re-registers) one DIoT runtime binding. `placement_id`
/// selects WHICH governed DIoT placement this binding is for; every other
/// identity field is derived exclusively from the active DMap/2 authority at
/// the moment of this call -- a caller cannot declare a `diot_id`, `node_id`,
/// DMap/DForward reference, or artifact authority of its own. Re-registering
/// the same `(project_id, application_id, placement_id)` is not an error: it
/// strictly increments `binding_revision` (starting at 1) and always
/// recomputes every derived field against CURRENT authority, so a binding
/// registered again after a topology change picks up the new authority
/// rather than silently keeping the old one.
#[allow(clippy::too_many_arguments)]
pub fn register_runtime_binding(
    state: &mut DIoTRuntimeBindingRegistryStateV1,
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    project_id: &str,
    application_id: &str,
    placement_id: &str,
    broker_uri: &str,
    registered_at_utc: &str,
) -> Result<DIoTRuntimeBindingV1, RegisterDIoTRuntimeBindingError> {
    let active = crate::core::ddeploy::active_placement_authority(
        ddeploy_state,
        project_id,
        application_id,
    )
    .map_err(RegisterDIoTRuntimeBindingError::NoActiveDMap2Authority)?
    .ok_or_else(|| {
        RegisterDIoTRuntimeBindingError::NoActiveDMap2Authority(format!(
            "project '{project_id}' application '{application_id}' has no active accepted D-Map"
        ))
    })?;
    let v2 = active.v2.as_ref().ok_or_else(|| {
        RegisterDIoTRuntimeBindingError::NoActiveDMap2Authority(format!(
            "project '{project_id}' application '{application_id}' active authority is DMap/1 -- DIoT runtime bindings require an active DMap/2"
        ))
    })?;

    let placement = v2
        .diot_placements
        .iter()
        .find(|p| p.placement_id == placement_id)
        .ok_or_else(|| {
            RegisterDIoTRuntimeBindingError::PlacementUnknown(format!(
                "placement '{placement_id}' is not a DIoT placement in the active DMap/2"
            ))
        })?;

    if !dnode_registry_state.nodes.contains_key(&placement.node_id) {
        return Err(RegisterDIoTRuntimeBindingError::NodeUnknown(format!(
            "DIoT placement '{placement_id}' names node '{}' which is not a currently declared structural D-Node",
            placement.node_id
        )));
    }

    let artifact_authority =
        current_diot_artifact_authority(ddeploy_state, project_id, application_id).ok_or_else(
            || {
                RegisterDIoTRuntimeBindingError::ArtifactAuthorityMissing(format!(
                    "no accepted proposal record found for the active DMap/2 authority of project '{project_id}' application '{application_id}'"
                ))
            },
        )?;
    let artifact_id = placement.artifact_binding.artifact_id.clone();
    let authority = artifact_authority.get(&artifact_id).ok_or_else(|| {
        RegisterDIoTRuntimeBindingError::ArtifactAuthorityMissing(format!(
            "DIoT placement '{placement_id}' artifact '{artifact_id}' has no resolved execution authority in the active DMap/2 acceptance"
        ))
    })?;

    validate_broker_uri(broker_uri).map_err(RegisterDIoTRuntimeBindingError::Invalid)?;

    let key = binding_key(project_id, application_id, placement_id);

    // Idempotent re-registration: if every field this function would derive
    // (everything except `registered_at_utc`, which is deliberately excluded
    // from this comparison so replaying the identical request never counts
    // as a "change") is already exactly what is persisted, return the
    // EXISTING record completely untouched -- no revision bump, no
    // `registered_at_utc` update, no digest recompute, no map mutation. This
    // keeps `binding_revision` a genuine change counter (broker_uri change,
    // or CURRENT authority moving) rather than a call counter.
    if let Some(existing) = state.bindings.get(&key) {
        let unchanged = existing.broker_uri == broker_uri
            && existing.diot_id == placement.diot_id
            && existing.node_id == placement.node_id
            && existing.dmap_id == active.dmap_id
            && existing.dmap_revision == active.revision
            && existing.dmap_digest == active.digest
            && existing.dforward_id == v2.dforward_id
            && existing.dforward_revision == v2.dforward_revision
            && existing.dforward_digest == v2.dforward_digest
            && existing.artifact_id == artifact_id
            && existing.execution_generation == authority.execution_generation
            && existing.execution_projection_digest == authority.execution_projection_digest;
        if unchanged {
            return Ok(existing.clone());
        }
    }

    let next_revision = state
        .bindings
        .get(&key)
        .map(|existing| existing.binding_revision + 1)
        .unwrap_or(1);
    let binding_id = format!("diot-runtime-binding:{project_id}:{application_id}:{placement_id}");
    let mut binding = DIoTRuntimeBindingV1 {
        binding_id,
        binding_revision: next_revision,
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        placement_id: placement_id.to_string(),
        diot_id: placement.diot_id.clone(),
        node_id: placement.node_id.clone(),
        dmap_id: active.dmap_id.clone(),
        dmap_revision: active.revision,
        dmap_digest: active.digest.clone(),
        dforward_id: v2.dforward_id.clone(),
        dforward_revision: v2.dforward_revision,
        dforward_digest: v2.dforward_digest.clone(),
        artifact_id,
        execution_generation: authority.execution_generation,
        execution_projection_digest: authority.execution_projection_digest.clone(),
        transport: TRANSPORT_MQTT.to_string(),
        broker_uri: broker_uri.to_string(),
        registered_at_utc: registered_at_utc.to_string(),
        descriptor_digest: String::new(),
    };
    binding.descriptor_digest = recompute_binding_digest(&binding).map_err(|error| {
        RegisterDIoTRuntimeBindingError::Invalid(finding(
            "diot_runtime_binding_digest_unrecomputable",
            error,
        ))
    })?;
    state.bindings.insert(key, binding.clone());
    Ok(binding)
}

/// Fail-closed staleness check: proves a previously-registered binding's
/// recorded authority is STILL the currently active one before any consumer
/// treats it as authoritative for dispatch. Mirrors
/// `dcall::check_source_execution_binding`'s exact-equality philosophy (K2,
/// Phase 112) -- a partial/approximate match is never acceptable for an
/// authority-continuity check. Never mutates the registry: staleness is
/// discovered, not repaired, here.
pub fn verify_runtime_binding_still_current(
    binding: &DIoTRuntimeBindingV1,
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
) -> Result<(), CanonicalValidationFinding> {
    let mismatch = |field: &str| {
        finding(
            "diot_runtime_binding_stale",
            format!("DIoT runtime binding is stale: {field} no longer matches the currently active DMap/2 authority"),
        )
    };

    let active = crate::core::ddeploy::active_placement_authority(
        ddeploy_state,
        &binding.project_id,
        &binding.application_id,
    )
    .map_err(|error| finding("diot_runtime_binding_stale", error))?
    .ok_or_else(|| {
        finding(
            "diot_runtime_binding_stale",
            format!(
                "project '{}' application '{}' no longer has an active accepted D-Map",
                binding.project_id, binding.application_id
            ),
        )
    })?;
    let v2 = active.v2.as_ref().ok_or_else(|| {
        finding(
            "diot_runtime_binding_stale",
            "active authority is no longer DMap/2".to_string(),
        )
    })?;

    if active.dmap_id != binding.dmap_id
        || active.revision != binding.dmap_revision
        || active.digest != binding.dmap_digest
    {
        return Err(mismatch("DMap ref"));
    }
    if v2.dforward_id != binding.dforward_id
        || v2.dforward_revision != binding.dforward_revision
        || v2.dforward_digest != binding.dforward_digest
    {
        return Err(mismatch("DForward ref"));
    }
    let placement = v2
        .diot_placements
        .iter()
        .find(|p| p.placement_id == binding.placement_id)
        .ok_or_else(|| {
            finding(
                "diot_runtime_binding_stale",
                format!(
                    "DIoT placement '{}' no longer exists in the active DMap/2",
                    binding.placement_id
                ),
            )
        })?;
    if placement.diot_id != binding.diot_id || placement.node_id != binding.node_id {
        return Err(mismatch("DIoT placement identity"));
    }
    if !dnode_registry_state.nodes.contains_key(&binding.node_id) {
        return Err(finding(
            "diot_runtime_binding_stale",
            format!(
                "node '{}' is no longer a declared structural D-Node",
                binding.node_id
            ),
        ));
    }
    let artifact_authority =
        current_diot_artifact_authority(ddeploy_state, &binding.project_id, &binding.application_id)
            .ok_or_else(|| {
                finding(
                    "diot_runtime_binding_stale",
                    "active authority's proposal record is missing".to_string(),
                )
            })?;
    let authority = artifact_authority
        .get(&binding.artifact_id)
        .ok_or_else(|| {
            finding(
                "diot_runtime_binding_stale",
                format!(
                    "artifact '{}' no longer has resolved execution authority",
                    binding.artifact_id
                ),
            )
        })?;
    if authority.execution_generation != binding.execution_generation
        || authority.execution_projection_digest != binding.execution_projection_digest
    {
        return Err(mismatch("artifact execution authority"));
    }
    Ok(())
}

/// Fail-closed relational integrity check, invoked from `load_state`.
/// Mirrors `dcall_endpoint`'s precedent: map-key consistency, each stored
/// binding's digest re-verified, `binding_revision` must be >= 1, and the
/// persisted `broker_uri`/`transport` are re-validated structurally (not
/// merely digest-recomputed) so a corrupted record whose digest was
/// recomputed to match cannot pass silently.
pub fn validate_persisted_diot_runtime_binding_registry_integrity(
    state: &DIoTRuntimeBindingRegistryStateV1,
) -> Result<(), String> {
    if state.state_version != DIOT_RUNTIME_BINDING_REGISTRY_STATE_VERSION {
        return Err(format!(
            "unsupported diot runtime binding registry state_version '{}', expected '{DIOT_RUNTIME_BINDING_REGISTRY_STATE_VERSION}'",
            state.state_version
        ));
    }
    for (key, binding) in &state.bindings {
        let expected_key =
            binding_key(&binding.project_id, &binding.application_id, &binding.placement_id);
        if *key != expected_key {
            return Err(format!(
                "diot runtime binding stored under key '{key}' does not match binding_key(...) = '{expected_key}'"
            ));
        }
        if binding.binding_revision < 1 {
            return Err(format!(
                "diot runtime binding '{key}' has binding_revision {} but must be >= 1",
                binding.binding_revision
            ));
        }
        if binding.transport != TRANSPORT_MQTT {
            return Err(format!(
                "diot runtime binding '{key}' has unsupported persisted transport '{}'",
                binding.transport
            ));
        }
        validate_broker_uri(&binding.broker_uri).map_err(|f| {
            format!(
                "diot runtime binding '{key}' fails structural re-validation: {}",
                f.message
            )
        })?;
        let expected_digest = recompute_binding_digest(binding).map_err(|error| {
            format!("diot runtime binding '{key}' descriptor digest could not be recomputed: {error}")
        })?;
        if expected_digest != binding.descriptor_digest {
            return Err(format!(
                "diot runtime binding '{key}' persisted descriptor_digest '{}' does not match recomputed '{expected_digest}' -- persisted state is corrupt or was tampered with",
                binding.descriptor_digest
            ));
        }
    }
    Ok(())
}

pub fn load_state(db_path: &std::path::Path) -> Result<DIoTRuntimeBindingRegistryStateV1, String> {
    match load_domain_json::<DIoTRuntimeBindingRegistryStateV1>(
        db_path,
        DOMAIN_DIOT_RUNTIME_BINDING_REGISTRY,
    )? {
        Some(state) => {
            validate_persisted_diot_runtime_binding_registry_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DIoTRuntimeBindingRegistryStateV1::empty()),
    }
}

pub fn persist_state(
    db_path: &std::path::Path,
    state: &DIoTRuntimeBindingRegistryStateV1,
) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DIOT_RUNTIME_BINDING_REGISTRY, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1,
        CanonicalNodeCapacityV1, CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dforward::{
        CanonicalArtifactRequirementV1, CanonicalDForwardInterfaceV1, CanonicalDForwardV1,
        CanonicalDIoTV1, CANONICAL_DFORWARD_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        recompute_dgraph_digest, CanonicalDCallV1, CanonicalDGraphV1,
        CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1,
        CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
    };
    use crate::core::canonical_dmap_v2::{
        ContractRefV2, DCallRealizationV2, DIoTArtifactBindingV2, DIoTPlacementV2,
        DServPlacementV2,
    };
    use crate::core::ddeploy::ProposalSource;
    use crate::core::ddeploy_v2::DDeployProposalV2;
    use crate::core::dnode_registry::declare_dnode;
    use crate::core::service_artifact_registry::{
        NativeProcessDeploymentSpecV1, ServiceArtifact, ServiceArtifactControlPlane,
        ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec, SERVICE_ARTIFACT_SCHEMA_VERSION,
    };
    use chrono::Utc;

    const PROJECT: &str = "proj:lab";
    const APP: &str = "app:pipeline";
    const SOURCE_SVC: &str = "sensor";
    const DEST_SVC: &str = "decoder";
    const CALL_ID: &str = "sensor-to-decoder";
    const DIOT_ID: &str = "diot:broker";
    const DIOT_PLACEMENT_ID: &str = "diotp:broker";
    const REALIZATION_INTERFACE: &str = "sensor-bus";
    const ARTIFACT_ID: &str = "dserv:pipeline:broker";

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

    fn minimal_broker_artifact(
        project_id: &str,
        execution_generation: u64,
        execution_projection_digest: &str,
    ) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: ARTIFACT_ID.to_string(),
            service_id: ARTIFACT_ID.to_string(),
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

    struct Fixture {
        ddeploy: DDeployStateV1,
        dnode: DNodeRegistryStateV1,
    }

    /// Builds one active DMap/2 with one DIoT broker placement realizing the
    /// fixture's one D-Call, on `broker_node`. Mirrors
    /// `dcall::tests::build_v2_fixture`'s shape (that helper is private to
    /// its own module and cannot be reused across files).
    fn build_fixture(broker_node: &str) -> Fixture {
        let mut ddeploy = DDeployStateV1::empty();
        seed_diot_fixture(
            &mut ddeploy,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "prop-diot-1",
            broker_node,
        );
        let mut dnode = DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode, node(broker_node)).unwrap();
        Fixture { ddeploy, dnode }
    }

    /// Generalized form of `build_fixture` parameterized by
    /// `project_id`/`application_id`/`placement_id`/`proposal_id`, so a
    /// SECOND real accepted DMap/2 authority for a DIFFERENT project can be
    /// seeded into the SAME `ddeploy` state (required for a genuine
    /// project-scope-isolation test -- see
    /// `registration_scoped_to_a_different_project_cannot_reuse_another_projects_placement`).
    fn seed_diot_fixture(
        ddeploy: &mut DDeployStateV1,
        project_id: &str,
        application_id: &str,
        placement_id: &str,
        proposal_id: &str,
        broker_node: &str,
    ) {
        let dg = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: application_id.to_string(),
            dgraph_id: format!("dgraph:{proposal_id}"),
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
        };
        let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();

        let dcontinuum = CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            dcontinuum_id: format!("dcontinuum:{proposal_id}"),
            revision: 1,
            nodes: vec![node(broker_node)],
            links: Vec::new(),
        };
        let dcontinuum_digest =
            crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum).unwrap();

        let dforward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            dforward_id: format!("dforward:{proposal_id}"),
            revision: 1,
            diots: vec![CanonicalDIoTV1 {
                diot_id: DIOT_ID.to_string(),
                diot_kind: "broker".to_string(),
                artifact_requirement: CanonicalArtifactRequirementV1 {
                    artifact_id: ARTIFACT_ID.to_string(),
                },
                provides: vec![CanonicalDForwardInterfaceV1 {
                    interface: REALIZATION_INTERFACE.to_string(),
                }],
                requires: Vec::new(),
            }],
            external_endpoints: Vec::new(),
            chains: Vec::new(),
            support_jobs: Vec::new(),
        };
        let dforward_digest = dforward.digest().unwrap();

        let proposal = DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: proposal_id.to_string(),
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: Utc::now().to_rfc3339(),
            dgraph_ref: ContractRefV2 {
                id: dg.dgraph_id.clone(),
                revision: dg.revision,
                digest: dgraph_digest,
            },
            dcontinuum_ref: ContractRefV2 {
                id: dcontinuum.dcontinuum_id.clone(),
                revision: dcontinuum.revision,
                digest: dcontinuum_digest,
            },
            dforward_ref: ContractRefV2 {
                id: dforward.dforward_id.clone(),
                revision: dforward.revision,
                digest: dforward_digest,
            },
            placements: vec![
                DServPlacementV2 {
                    service_id: SOURCE_SVC.to_string(),
                    node_id: broker_node.to_string(),
                },
                DServPlacementV2 {
                    service_id: DEST_SVC.to_string(),
                    node_id: broker_node.to_string(),
                },
            ],
            diot_placements: vec![DIoTPlacementV2 {
                placement_id: placement_id.to_string(),
                diot_id: DIOT_ID.to_string(),
                node_id: broker_node.to_string(),
                artifact_binding: DIoTArtifactBindingV2 {
                    artifact_id: ARTIFACT_ID.to_string(),
                    descriptor_digest: format!("sha256:{}", "b".repeat(64)),
                    content_digest: None,
                },
            }],
            dcall_realizations: vec![DCallRealizationV2 {
                call_id: CALL_ID.to_string(),
                realization_kind: "via_diot_pubsub".to_string(),
                diot_placement_id: placement_id.to_string(),
                interface: REALIZATION_INTERFACE.to_string(),
                channel: "pipeline/sensor-to-decoder".to_string(),
            }],
            external_port_bindings: Vec::new(),
            expected_active_dmap_revision: None,
            diot_artifact_authority: {
                let mut m = BTreeMap::new();
                m.insert(
                    ARTIFACT_ID.to_string(),
                    DIoTArtifactAuthorityBindingV2 {
                        artifact_id: ARTIFACT_ID.to_string(),
                        execution_generation: 1,
                        execution_projection_digest: format!("sha256:{}", "c".repeat(64)),
                    },
                );
                m
            },
        };

        let broker_artifact =
            minimal_broker_artifact(project_id, 1, &format!("sha256:{}", "c".repeat(64)));

        ddeploy
            .create_v2_proposal(proposal, dg, dcontinuum.clone(), dforward)
            .unwrap();
        ddeploy
            .accept_v2_proposal(
                proposal_id,
                format!("acc-{proposal_id}"),
                "test".to_string(),
                "2026-01-01T00:00:01Z".to_string(),
                &dcontinuum,
                std::slice::from_ref(&broker_artifact),
                None,
            )
            .unwrap();
    }

    // ---------------------------------------------------------- REGISTRATION

    #[test]
    fn registration_succeeds_at_revision_one() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        assert_eq!(binding.binding_revision, 1);
        assert_eq!(binding.diot_id, DIOT_ID);
        assert_eq!(binding.node_id, "fog-01");
        assert_eq!(binding.transport, TRANSPORT_MQTT);
        assert_eq!(binding.artifact_id, ARTIFACT_ID);
        assert_eq!(binding.execution_generation, 1);
        assert!(!binding.descriptor_digest.is_empty());
    }

    #[test]
    fn registration_derives_dmap_and_dforward_identity_never_caller_supplied() {
        let fixture = build_fixture("fog-01");
        let active = crate::core::ddeploy::active_placement_authority(&fixture.ddeploy, PROJECT, APP)
            .unwrap()
            .unwrap();
        let v2 = active.v2.as_ref().unwrap();
        let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
        let binding = register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        assert_eq!(binding.dmap_id, active.dmap_id);
        assert_eq!(binding.dmap_revision, active.revision);
        assert_eq!(binding.dmap_digest, active.digest);
        assert_eq!(binding.dforward_id, v2.dforward_id);
        assert_eq!(binding.dforward_revision, v2.dforward_revision);
        assert_eq!(binding.dforward_digest, v2.dforward_digest);
    }

    #[test]
    fn reregistration_increments_revision_and_updates_broker_uri() {
        let fixture = build_fixture("fog-01");
        let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
        register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let updated = register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker2.lab:1883",
            "2026-01-01T00:00:03Z",
        )
        .unwrap();
        assert_eq!(updated.binding_revision, 2);
        assert_eq!(updated.broker_uri, "mqtt://broker2.lab:1883");
    }

    /// Section 3 closure: registering the SAME project/application/placement
    /// with the SAME broker_uri under the SAME (unchanged) accepted
    /// authority must be a genuine no-op -- not merely "no error", but
    /// exactly the same binding_id/binding_revision/descriptor_digest/
    /// authority fields, and no duplicate registry entry. Must not be
    /// confused with `reregistration_increments_revision_and_updates_broker_uri`
    /// above, which deliberately changes broker_uri and must keep
    /// incrementing.
    #[test]
    fn identical_reregistration_is_idempotent() {
        let fixture = build_fixture("fog-01");
        let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
        let first = register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let second = register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:09Z",
        )
        .unwrap();
        assert_eq!(second.binding_id, first.binding_id);
        assert_eq!(second.binding_revision, first.binding_revision);
        assert_eq!(second.descriptor_digest, first.descriptor_digest);
        assert_eq!(second.registered_at_utc, first.registered_at_utc);
        assert_eq!(second.dmap_id, first.dmap_id);
        assert_eq!(second.dmap_revision, first.dmap_revision);
        assert_eq!(second.dmap_digest, first.dmap_digest);
        assert_eq!(second.dforward_id, first.dforward_id);
        assert_eq!(second.dforward_revision, first.dforward_revision);
        assert_eq!(second.dforward_digest, first.dforward_digest);
        assert_eq!(second.artifact_id, first.artifact_id);
        assert_eq!(second.execution_generation, first.execution_generation);
        assert_eq!(
            second.execution_projection_digest,
            first.execution_projection_digest
        );
        assert_eq!(state.bindings.len(), 1);
    }

    /// Section 4 closure: two REAL, independently-accepted DMap/2
    /// authorities coexist in the SAME `DDeployStateV1` -- one for `PROJECT`
    /// (whose DIoT placement id is `DIOT_PLACEMENT_ID`), one for a SECOND
    /// project that has its OWN, differently-named DIoT placement. A
    /// registration call scoped to the second project must not be able to
    /// read, reuse, or register `PROJECT`'s placement id -- it must resolve
    /// strictly against the second project's OWN active authority (the real
    /// production `(project_id, application_id)` scoping key), so the
    /// lookup fails with `PlacementUnknown`, not by accident but because the
    /// two projects' placement identities are genuinely disjoint.
    #[test]
    fn registration_scoped_to_a_different_project_cannot_reuse_another_projects_placement() {
        const OTHER_PROJECT: &str = "proj:other";
        const OTHER_PLACEMENT: &str = "diotp:other-broker";
        let mut ddeploy = DDeployStateV1::empty();
        seed_diot_fixture(&mut ddeploy, PROJECT, APP, DIOT_PLACEMENT_ID, "prop-diot-1", "fog-01");
        seed_diot_fixture(
            &mut ddeploy,
            OTHER_PROJECT,
            APP,
            OTHER_PLACEMENT,
            "prop-diot-2",
            "fog-01",
        );
        let mut dnode = DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode, node("fog-01")).unwrap();

        let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
        let error = register_runtime_binding(
            &mut state,
            &ddeploy,
            &dnode,
            OTHER_PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RegisterDIoTRuntimeBindingError::PlacementUnknown(_)
        ));
        assert!(state.bindings.is_empty());

        // Sanity: the SAME placement id genuinely does resolve under its own
        // project, and the other project's own placement id resolves too --
        // proving this is scoping, not a broken fixture.
        register_runtime_binding(
            &mut state,
            &ddeploy,
            &dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        register_runtime_binding(
            &mut state,
            &ddeploy,
            &dnode,
            OTHER_PROJECT,
            APP,
            OTHER_PLACEMENT,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        assert_eq!(state.bindings.len(), 2);
    }

    /// Section 5 case B: the node named by the ACCEPTED DIoT placement
    /// ("fog-01") is genuinely NOT among the structurally-declared D-Nodes,
    /// while a DIFFERENT node ("cloud-01") IS structurally declared. This is
    /// an authority MISMATCH (the placement belongs to a node other than any
    /// currently known one), distinct from
    /// `registration_for_node_not_structurally_declared_is_rejected` (case
    /// A) above, which uses a completely EMPTY D-Node registry. Both must
    /// fail closed with the same `NodeUnknown` error class, and neither may
    /// create or update a runtime binding.
    #[test]
    fn registration_fails_when_placements_node_differs_from_the_only_declared_node() {
        let fixture = build_fixture("fog-01");
        let mut dnode = DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode, node("cloud-01")).unwrap();
        let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
        let error = register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RegisterDIoTRuntimeBindingError::NodeUnknown(_)
        ));
        assert!(state.bindings.is_empty());
    }

    #[test]
    fn registration_for_unknown_placement_is_rejected() {
        let fixture = build_fixture("fog-01");
        let error = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            "diotp:unknown",
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RegisterDIoTRuntimeBindingError::PlacementUnknown(_)
        ));
    }

    #[test]
    fn registration_without_active_dmap2_authority_is_rejected() {
        let error = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &DDeployStateV1::empty(),
            &DNodeRegistryStateV1::empty(),
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RegisterDIoTRuntimeBindingError::NoActiveDMap2Authority(_)
        ));
    }

    #[test]
    fn registration_for_node_not_structurally_declared_is_rejected() {
        let fixture = build_fixture("fog-01");
        let error = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &DNodeRegistryStateV1::empty(),
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            RegisterDIoTRuntimeBindingError::NodeUnknown(_)
        ));
    }

    #[test]
    fn non_mqtt_scheme_is_rejected() {
        let fixture = build_fixture("fog-01");
        let error = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "http://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(
            matches!(error, RegisterDIoTRuntimeBindingError::Invalid(f) if f.code == "diot_runtime_binding_broker_scheme_unsupported")
        );
    }

    #[test]
    fn userinfo_in_broker_uri_is_rejected() {
        let fixture = build_fixture("fog-01");
        let error = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://user:pass@broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap_err();
        assert!(
            matches!(error, RegisterDIoTRuntimeBindingError::Invalid(f) if f.code == "diot_runtime_binding_broker_uri_userinfo_rejected")
        );
    }

    #[test]
    fn query_and_fragment_broker_uris_are_rejected_without_registry_mutation() {
        for (uri, code) in [
            (
                "mqtt://broker.lab:1883?x=1",
                "diot_runtime_binding_broker_uri_query_rejected",
            ),
            (
                "mqtt://broker.lab:1883#frag",
                "diot_runtime_binding_broker_uri_fragment_rejected",
            ),
        ] {
            let fixture = build_fixture("fog-01");
            let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
            let error = register_runtime_binding(
                &mut state,
                &fixture.ddeploy,
                &fixture.dnode,
                PROJECT,
                APP,
                DIOT_PLACEMENT_ID,
                uri,
                "2026-01-01T00:00:02Z",
            )
            .unwrap_err();
            assert!(matches!(error, RegisterDIoTRuntimeBindingError::Invalid(f) if f.code == code));
            assert!(state.bindings.is_empty());
        }
    }

    /// Section 6 closure: the actual v0.1 broker-URI grammar
    /// (`mqtt://host:port`, no userinfo) rejects more than just the
    /// userinfo/query/fragment cases already covered above. Every malformed
    /// case the directive names is exercised here, each independently
    /// proving both rejection AND that no registry mutation occurs.
    #[test]
    fn malformed_broker_uris_are_rejected_without_registry_mutation() {
        for uri in [
            "",
            "localhost:1883",
            "http://localhost:1883",
            "mqtt://",
            "mqtt:///1883",
        ] {
            let fixture = build_fixture("fog-01");
            let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
            let error = register_runtime_binding(
                &mut state,
                &fixture.ddeploy,
                &fixture.dnode,
                PROJECT,
                APP,
                DIOT_PLACEMENT_ID,
                uri,
                "2026-01-01T00:00:02Z",
            )
            .unwrap_err();
            assert!(
                matches!(error, RegisterDIoTRuntimeBindingError::Invalid(_)),
                "broker_uri {uri:?} must be rejected as invalid, got {error:?}"
            );
            assert!(state.bindings.is_empty(), "broker_uri {uri:?} must not mutate the registry");
        }
    }

    /// Section 7 closure: the descriptor_digest is server-owned and
    /// deterministic -- recomputing it from the SAME semantic binding
    /// content (independently of the original registration call) yields the
    /// EXACT same digest, and a caller-supplied/tampered digest can never
    /// substitute for it.
    #[test]
    fn descriptor_digest_recomputation_is_deterministic() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let recomputed_once = recompute_binding_digest(&binding).unwrap();
        let recomputed_twice = recompute_binding_digest(&binding).unwrap();
        assert_eq!(binding.descriptor_digest, recomputed_once);
        assert_eq!(recomputed_once, recomputed_twice);

        // A caller-tampered digest is never accepted as a substitute: it
        // fails persisted integrity validation on reload rather than being
        // trusted (this is the write-side half of
        // `tampered_descriptor_digest_fails_reload`'s reload-side proof --
        // the digest is server-recomputed, never caller-selected, at both
        // ends of the persistence boundary).
        let mut tampered = binding.clone();
        tampered.descriptor_digest = format!("sha256:{}", "0".repeat(64));
        assert_ne!(tampered.descriptor_digest, recomputed_once);
        let mut state = DIoTRuntimeBindingRegistryStateV1::empty();
        state
            .bindings
            .insert(binding_key(PROJECT, APP, DIOT_PLACEMENT_ID), tampered);
        let error = validate_persisted_diot_runtime_binding_registry_integrity(&state).unwrap_err();
        assert!(error.contains("does not match recomputed"));
    }

    // ------------------------------------------------------------ STALENESS

    #[test]
    fn fresh_binding_is_not_stale() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        verify_runtime_binding_still_current(&binding, &fixture.ddeploy, &fixture.dnode).unwrap();
    }

    #[test]
    fn binding_is_stale_after_dmap_revision_moves() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
        active.map.revision += 1;
        active.digest = active.map.digest().unwrap();
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    /// Section 11, dimension 2: the DMap's `revision` scalar stays UNCHANGED
    /// but its content (an unrelated DServ placement, not the DIoT
    /// placement or DForward ref this binding cares about) changes, so only
    /// `active.digest` differs from the persisted `dmap_digest` -- proving
    /// the digest half of the DMap-ref check is independently load-bearing,
    /// not merely a byproduct of the revision check
    /// (`binding_is_stale_after_dmap_revision_moves` above only proves the
    /// revision half).
    #[test]
    fn binding_is_stale_after_dmap_digest_changes_without_a_revision_bump() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
        let original_revision = active.map.revision;
        for placement in active.map.placements.iter_mut() {
            if placement.service_id == SOURCE_SVC {
                placement.node_id = "cloud-99".to_string();
            }
        }
        active.digest = active.map.digest().unwrap();
        assert_eq!(
            active.map.revision, original_revision,
            "this test must isolate a digest change from a revision change"
        );
        assert_ne!(
            active.digest, binding.dmap_digest,
            "the DServ placement content change must actually move the digest"
        );
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    /// Section 8 closure: the accepted DForward reference (id/revision/
    /// digest) changes while the DMap's own id/revision and its OWN stored
    /// digest are left exactly as they were at registration -- isolating
    /// this from `binding_is_stale_after_dmap_revision_moves` and the
    /// digest-only test above, both of which trip the OUTER "DMap ref"
    /// check. `dforward_ref` is embedded inside `CanonicalDMapV2` itself, so
    /// in a REAL acceptance any DForward change necessarily also moves the
    /// DMap's own digest; deliberately leaving `active.digest` untouched
    /// here (mirroring
    /// `v2_route_derivation_fails_closed_when_active_dmap2_is_missing_a_realization_for_the_call`'s
    /// own "force an otherwise-unreachable state directly on in-memory
    /// active authority" technique) isolates the INNER "DForward ref" branch
    /// so it is proven independently rather than always being shadowed by
    /// the outer DMap-ref check.
    #[test]
    fn binding_is_stale_after_dforward_authority_changes() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
        active.map.dforward_ref.revision += 1;
        // `active.digest` is deliberately NOT recomputed -- see doc comment.
        assert_eq!(
            active.map.dmap_id, binding.dmap_id,
            "DMap identity must stay unchanged so the OUTER DMap-ref check passes"
        );
        assert_eq!(
            active.map.revision, binding.dmap_revision,
            "DMap revision must stay unchanged so the OUTER DMap-ref check passes"
        );
        assert_eq!(
            active.digest, binding.dmap_digest,
            "DMap digest must stay unchanged so this isolates the DForward-ref check"
        );
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    /// Section 9 closure: the ServiceArtifact's `execution_projection_digest`
    /// changes while `execution_generation` stays the same -- must be
    /// distinct from the `execution_generation`-change case
    /// (`binding_is_stale_after_artifact_execution_generation_changes`
    /// below), since the production check ORs both fields together and a
    /// test that only ever changes both at once could not prove either is
    /// independently load-bearing.
    #[test]
    fn binding_is_stale_after_artifact_execution_projection_digest_changes() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let proposal_id = ddeploy.phase113_v2.active.get(&key).unwrap().proposal_id.clone();
        let record = ddeploy.phase113_v2.proposals.get_mut(&proposal_id).unwrap();
        let authority = record
            .proposal
            .diot_artifact_authority
            .get_mut(ARTIFACT_ID)
            .unwrap();
        assert_eq!(authority.execution_generation, binding.execution_generation);
        authority.execution_projection_digest = format!("sha256:{}", "d".repeat(64));
        assert_ne!(
            authority.execution_projection_digest,
            binding.execution_projection_digest
        );
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    /// Section 10 closure: the ServiceArtifact's `execution_generation`
    /// changes while `execution_projection_digest` stays the same -- the
    /// counterpart to the projection-digest test above.
    #[test]
    fn binding_is_stale_after_artifact_execution_generation_changes() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let proposal_id = ddeploy.phase113_v2.active.get(&key).unwrap().proposal_id.clone();
        let record = ddeploy.phase113_v2.proposals.get_mut(&proposal_id).unwrap();
        let authority = record
            .proposal
            .diot_artifact_authority
            .get_mut(ARTIFACT_ID)
            .unwrap();
        assert_eq!(
            authority.execution_projection_digest,
            binding.execution_projection_digest
        );
        authority.execution_generation += 1;
        assert_ne!(authority.execution_generation, binding.execution_generation);
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    #[test]
    fn binding_is_stale_after_placement_moves_to_a_different_node() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let mut dnode = fixture.dnode.clone();
        declare_dnode(&mut dnode, node("cloud-01")).unwrap();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
        for placement in active.map.diot_placements.iter_mut() {
            if placement.placement_id == DIOT_PLACEMENT_ID {
                placement.node_id = "cloud-01".to_string();
            }
        }
        active.digest = active.map.digest().unwrap();
        let error = verify_runtime_binding_still_current(&binding, &ddeploy, &dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    #[test]
    fn binding_is_stale_after_placement_removed_from_active_dmap() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        let active = ddeploy.phase113_v2.active.get_mut(&key).unwrap();
        active.map.diot_placements.clear();
        active.digest = active.map.digest().unwrap();
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    #[test]
    fn binding_is_stale_when_active_authority_reverts_to_dmap1() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let mut ddeploy = fixture.ddeploy.clone();
        let key = crate::core::ddeploy::application_key(PROJECT, APP);
        ddeploy.phase113_v2.active.remove(&key);
        let error =
            verify_runtime_binding_still_current(&binding, &ddeploy, &fixture.dnode).unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    #[test]
    fn binding_is_stale_when_node_is_no_longer_structurally_declared() {
        let fixture = build_fixture("fog-01");
        let binding = register_runtime_binding(
            &mut DIoTRuntimeBindingRegistryStateV1::empty(),
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let error = verify_runtime_binding_still_current(
            &binding,
            &fixture.ddeploy,
            &DNodeRegistryStateV1::empty(),
        )
        .unwrap_err();
        assert_eq!(error.code, "diot_runtime_binding_stale");
    }

    // ---------------------------------------------------------- PERSISTENCE

    fn tempfile_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "diot-runtime-binding-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn persisted_reload_roundtrips() {
        let fixture = build_fixture("fog-01");
        let dir = tempfile_dir();
        let db_path = dir.join("control-state.sqlite3");
        let mut state = load_state(&db_path).unwrap();
        register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        persist_state(&db_path, &state).unwrap();
        let reloaded = load_state(&db_path).unwrap();
        assert_eq!(reloaded, state);
    }

    #[test]
    fn tampered_descriptor_digest_fails_reload() {
        let fixture = build_fixture("fog-01");
        let dir = tempfile_dir();
        let db_path = dir.join("control-state.sqlite3");
        let mut state = load_state(&db_path).unwrap();
        register_runtime_binding(
            &mut state,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        let key = binding_key(PROJECT, APP, DIOT_PLACEMENT_ID);
        state.bindings.get_mut(&key).unwrap().broker_uri = "mqtt://tampered.lab:1883".to_string();
        persist_state(&db_path, &state).unwrap();
        let error = load_state(&db_path).unwrap_err();
        assert!(error.contains("does not match recomputed"));
    }
}
