//! Phase 119B: a small, pure, NON-AUTHORITATIVE core that relates an exact
//! source placement-authority reference to an exact target D-Deploy proposal
//! reference, and derives their CURRENT relationship from already-existing,
//! already-persisted D-Deploy state.
//!
//! This module owns nothing. It never selects a target, never mutates any
//! D-Deploy/D-Map/D-Node/D-Code/D-Monitor state, and never persists
//! anything of its own. It exists solely to answer, deterministically and
//! read-only: "given this exact source authority reference and this exact
//! target proposal reference, what is their relationship right now?" --
//! see `documentation/reference/phase119b-migration-intent-authority-relation-core.md`
//! for the full audit trail behind every decision made here, in particular
//! why no persisted `MigrationRecord`/workflow state machine exists (Phase
//! 119A's own recommendation, refined by this phase's own task framing:
//! facts like "content prepared"/"cleaning up" cannot be honestly derived
//! from canonical state alone, so this module never invents them).
//!
//! `MigrationIntentV1` is deliberately NOT a canonical `datum.*` wire
//! schema: it carries no `schema` field, is never submitted/validated/
//! accepted the way a D-Deploy proposal is, and is never persisted. Its
//! `Serialize`/`Deserialize`/`PartialEq` derives exist purely for test and
//! internal-caller ergonomics, exactly like `PlacementAuthorityView` itself
//! (Phase 117/118) carries no `schema` field and is never treated as wire
//! authority either.

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::ddeploy::{self, DDeployStateV1, ProposalLifecycleState};
use crate::core::ddeploy_v2::{self, ProposalV2State};

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

fn digest_ok(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|x| {
        x.len() == 64 && x.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

pub const F_MIGRATION_INTENT_MALFORMED: &str = "migration_intent_malformed";
pub const F_MIGRATION_TARGET_PROPOSAL_NOT_FOUND: &str = "migration_target_proposal_not_found";
pub const F_MIGRATION_TARGET_PROPOSAL_DIGEST_CORRUPTED: &str =
    "migration_target_proposal_digest_corrupted";
pub const F_MIGRATION_TARGET_PROPOSAL_DIGEST_MISMATCH: &str =
    "migration_target_proposal_digest_mismatch";
pub const F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH: &str =
    "migration_target_proposal_scope_mismatch";
pub const F_MIGRATION_TARGET_ACCEPTANCE_MISSING: &str = "migration_target_acceptance_missing";
pub const F_MIGRATION_SOURCE_AUTHORITY_CORRUPTED: &str = "migration_source_authority_corrupted";

/// A pure reference/intent object: "migrate this exact current placement
/// authority to this exact already-existing target proposal." It duplicates
/// NOTHING -- not placements, not D-Code bindings, not D-Forward/D-IoT
/// content, not proposal lifecycle. `source_dmap_ref` reuses
/// `CanonicalContractReferenceV1` (the same `{id, revision, digest}` shape
/// `PlacementAuthorityView` itself exposes) rather than inventing a second
/// reference type. The target is identified by its own two real identity
/// fields (`proposal_id`, `proposal_digest`) -- never by a fabricated D-Map
/// reference, since D-Map identity/revision are server-owned and only
/// materialize at acceptance (Phase 119A §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationIntentV1 {
    pub project_id: String,
    pub application_id: String,
    pub source_dmap_ref: CanonicalContractReferenceV1,
    pub target_proposal_id: String,
    pub target_proposal_digest: String,
}

/// The CURRENT, derived relationship between a `MigrationIntentV1` and
/// existing D-Deploy state. Every variant is a fact this module can
/// genuinely prove from already-persisted state -- none imply runtime
/// preparation, health, convergence, or cleanup completion (see the module
/// doc and the phase document's "what 119B cannot know" section).
#[derive(Debug, Clone, PartialEq)]
pub enum MigrationAuthorityRelation {
    /// The target proposal is still `Pending`, its own
    /// `expected_active_dmap_revision` matches `intent.source_dmap_ref`'s
    /// revision, and the CURRENT active authority is exactly
    /// `intent.source_dmap_ref`. Nothing prevents requesting acceptance of
    /// the target proposal through the ordinary, unmodified D-Deploy
    /// acceptance path.
    ReadyForAcceptance,
    /// No active accepted D-Map exists at all for this project/application
    /// (`active_placement_authority` returned `Ok(None)`). There is no
    /// source to migrate away from.
    NoActiveSource,
    /// The current active authority no longer matches
    /// `intent.source_dmap_ref` (different `dmap_id`/`revision`/`digest`),
    /// and the target proposal was NOT the cause (see
    /// `TargetAlreadyAccepted`, which is always checked first and takes
    /// priority). Some other acceptance won since the intent's source
    /// reference was captured.
    SourceStale {
        current_source: CanonicalContractReferenceV1,
        expected_source: CanonicalContractReferenceV1,
    },
    /// The target proposal's own `expected_active_dmap_revision` does not
    /// match `intent.source_dmap_ref.revision`, even though the target
    /// proposal is still `Pending` and the current active authority still
    /// matches `intent.source_dmap_ref` exactly. The intent, as
    /// constructed, is internally inconsistent with its own target
    /// proposal's acceptance precondition.
    TargetInvalidForSource {
        target_expected_active_dmap_revision: Option<u64>,
        source_revision: u64,
    },
    /// The exact target proposal (`target_proposal_id`, digest-verified)
    /// has ALREADY been accepted -- checked directly from the proposal's
    /// own persisted acceptance evidence, independent of whatever the
    /// CURRENT active authority happens to be right now. This is the
    /// answer a crash-after-acceptance re-evaluation must reach, and the
    /// answer that correctly distinguishes "this exact migration's target
    /// won" from "some unrelated proposal won" (Phase 119A §28,
    /// this phase's crash-recovery/idempotency requirements). This is a
    /// legitimate, expected relation, never an error -- authority cutover
    /// having occurred says nothing about runtime preparation, cleanup, or
    /// operational convergence (Phase 119A §32), which this module cannot
    /// and does not claim to know.
    TargetAlreadyAccepted {
        materialized_dmap_ref: CanonicalContractReferenceV1,
    },
}

/// A normalized, v1/v2-neutral view of exactly the facts this module needs
/// from a target proposal record. Never serialized, never persisted, never
/// returned to a caller -- purely an internal seam so
/// `evaluate_migration_authority_relation` contains exactly one decision
/// tree instead of two parallel ones (Phase 119A §9's "avoid duplicated
/// v1/v2 validation logic").
struct NormalizedTargetProposal {
    project_id: String,
    application_id: String,
    stored_digest: String,
    recomputed_digest: Result<String, String>,
    expected_active_dmap_revision: Option<u64>,
    accepted: bool,
    accepted_acceptance_id: Option<String>,
}

/// V1 is checked first, then v2, deterministically -- the two proposal
/// stores are populated by entirely separate creation paths
/// (`create_proposal` vs. `DDeployStateV2::create_proposal`) and nothing in
/// the existing codebase suggests the same `proposal_id` string could ever
/// legitimately appear in both; this function does not additionally guard
/// against that (undemonstrated) case.
fn locate_target_proposal(
    state: &DDeployStateV1,
    proposal_id: &str,
) -> Option<NormalizedTargetProposal> {
    if let Some(record) = state.proposals.get(proposal_id) {
        return Some(NormalizedTargetProposal {
            project_id: record.proposal.project_id.clone(),
            application_id: record.proposal.application_id.clone(),
            stored_digest: record.proposal_digest.clone(),
            recomputed_digest: ddeploy::compute_proposal_digest(&record.proposal),
            expected_active_dmap_revision: record.proposal.expected_active_dmap_revision,
            accepted: record.state == ProposalLifecycleState::Accepted,
            accepted_acceptance_id: record.accepted_acceptance_id.clone(),
        });
    }
    if let Some(record) = state.phase113_v2.proposals.get(proposal_id) {
        return Some(NormalizedTargetProposal {
            project_id: record.proposal.project_id.clone(),
            application_id: record.proposal.application_id.clone(),
            stored_digest: record.proposal_digest.clone(),
            recomputed_digest: ddeploy_v2::DDeployStateV2::proposal_digest(&record.proposal),
            expected_active_dmap_revision: record.proposal.expected_active_dmap_revision,
            accepted: record.state == ProposalV2State::Accepted,
            accepted_acceptance_id: record.accepted_acceptance_id.clone(),
        });
    }
    None
}

fn lookup_materialized_dmap_ref(
    state: &DDeployStateV1,
    acceptance_id: &str,
) -> Option<CanonicalContractReferenceV1> {
    if let Some(acceptance) = state.acceptances.get(acceptance_id) {
        return Some(acceptance.materialized_dmap_ref.clone());
    }
    if let Some(acceptance) = state.phase113_v2.acceptances.get(acceptance_id) {
        let r = &acceptance.materialized_dmap_ref;
        return Some(CanonicalContractReferenceV1 {
            id: r.id.clone(),
            revision: r.revision,
            digest: r.digest.clone(),
        });
    }
    None
}

fn validate_intent_shape(intent: &MigrationIntentV1) -> Result<(), CanonicalValidationFinding> {
    if intent.project_id.trim().is_empty() {
        return Err(finding(F_MIGRATION_INTENT_MALFORMED, "project_id must be non-empty"));
    }
    if intent.application_id.trim().is_empty() {
        return Err(finding(F_MIGRATION_INTENT_MALFORMED, "application_id must be non-empty"));
    }
    if intent.source_dmap_ref.id.trim().is_empty() {
        return Err(finding(F_MIGRATION_INTENT_MALFORMED, "source_dmap_ref.id must be non-empty"));
    }
    if !digest_ok(&intent.source_dmap_ref.digest) {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "source_dmap_ref.digest must be a canonical 'sha256:<64 lowercase hex>' digest",
        ));
    }
    if intent.target_proposal_id.trim().is_empty() {
        return Err(finding(F_MIGRATION_INTENT_MALFORMED, "target_proposal_id must be non-empty"));
    }
    if !digest_ok(&intent.target_proposal_digest) {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "target_proposal_digest must be a canonical 'sha256:<64 lowercase hex>' digest",
        ));
    }
    Ok(())
}

/// The sole entry point. Pure and read-only: takes `state` by shared
/// reference, never mutates it, never calls `accept_proposal`/
/// `accept_with_previous`, never touches D-Monitor, D-Code, D-Call, or
/// D-Node registry state, and never selects or scores a target -- `intent`
/// already names its exact target. See the module doc and
/// `documentation/reference/phase119b-migration-intent-authority-relation-core.md`
/// for the full decision-tree rationale.
pub fn evaluate_migration_authority_relation(
    state: &DDeployStateV1,
    intent: &MigrationIntentV1,
) -> Result<MigrationAuthorityRelation, CanonicalValidationFinding> {
    validate_intent_shape(intent)?;

    let target = locate_target_proposal(state, &intent.target_proposal_id).ok_or_else(|| {
        finding(
            F_MIGRATION_TARGET_PROPOSAL_NOT_FOUND,
            format!("target proposal '{}' does not exist in the supplied D-Deploy state", intent.target_proposal_id),
        )
    })?;

    let recomputed = target.recomputed_digest.as_deref().map_err(|error| {
        finding(
            F_MIGRATION_TARGET_PROPOSAL_DIGEST_CORRUPTED,
            format!("target proposal '{}' digest could not be recomputed: {error}", intent.target_proposal_id),
        )
    })?;
    if recomputed != target.stored_digest {
        return Err(finding(
            F_MIGRATION_TARGET_PROPOSAL_DIGEST_CORRUPTED,
            format!(
                "target proposal '{}' recomputed digest '{recomputed}' does not match its own stored digest '{}' -- persisted proposal state is corrupted",
                intent.target_proposal_id, target.stored_digest
            ),
        ));
    }
    if target.stored_digest != intent.target_proposal_digest {
        return Err(finding(
            F_MIGRATION_TARGET_PROPOSAL_DIGEST_MISMATCH,
            format!(
                "target proposal '{}' actual digest '{}' does not match the digest '{}' asserted by the migration intent",
                intent.target_proposal_id, target.stored_digest, intent.target_proposal_digest
            ),
        ));
    }

    if target.project_id != intent.project_id || target.application_id != intent.application_id {
        return Err(finding(
            F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH,
            format!(
                "target proposal '{}' belongs to project '{}' application '{}', not the intent's project '{}' application '{}'",
                intent.target_proposal_id, target.project_id, target.application_id, intent.project_id, intent.application_id
            ),
        ));
    }

    // Checked BEFORE evaluating source currency, deliberately: this is the
    // property that makes crash-after-acceptance recovery and the
    // "unrelated proposal won" distinction correct (Phase 119A §28; this
    // phase's own crash-recovery requirement). Whatever the current active
    // authority is right now, if THIS exact target proposal was accepted,
    // that fact is independently, permanently true and must be reported as
    // such rather than mis-derived as "the source went stale."
    if target.accepted {
        let acceptance_id = target.accepted_acceptance_id.as_deref().ok_or_else(|| {
            finding(
                F_MIGRATION_TARGET_ACCEPTANCE_MISSING,
                format!("target proposal '{}' is marked Accepted but carries no accepted_acceptance_id", intent.target_proposal_id),
            )
        })?;
        let materialized_dmap_ref = lookup_materialized_dmap_ref(state, acceptance_id).ok_or_else(|| {
            finding(
                F_MIGRATION_TARGET_ACCEPTANCE_MISSING,
                format!("target proposal '{}' references acceptance '{acceptance_id}', which does not exist in the supplied D-Deploy state", intent.target_proposal_id),
            )
        })?;
        return Ok(MigrationAuthorityRelation::TargetAlreadyAccepted { materialized_dmap_ref });
    }

    match ddeploy::active_placement_authority(state, &intent.project_id, &intent.application_id) {
        Err(error) => Err(finding(F_MIGRATION_SOURCE_AUTHORITY_CORRUPTED, error)),
        Ok(None) => Ok(MigrationAuthorityRelation::NoActiveSource),
        Ok(Some(view)) => {
            let current_source = CanonicalContractReferenceV1 {
                id: view.dmap_id,
                revision: view.revision,
                digest: view.digest,
            };
            if current_source != intent.source_dmap_ref {
                return Ok(MigrationAuthorityRelation::SourceStale {
                    current_source,
                    expected_source: intent.source_dmap_ref.clone(),
                });
            }
            if target.expected_active_dmap_revision != Some(intent.source_dmap_ref.revision) {
                return Ok(MigrationAuthorityRelation::TargetInvalidForSource {
                    target_expected_active_dmap_revision: target.expected_active_dmap_revision,
                    source_revision: intent.source_dmap_ref.revision,
                });
            }
            Ok(MigrationAuthorityRelation::ReadyForAcceptance)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1,
        CanonicalNodeCapacityV1, CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        CanonicalDGraphV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1,
        CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
    };
    use crate::core::canonical_dmap::CanonicalDServPlacementV1;
    use crate::core::ddeploy::{accept_proposal, create_proposal, AcceptOutcome, CreateProposalInput, ProposalSource};
    use crate::core::service_artifact_registry::{
        ServiceArtifact, ServiceArtifactControlPlane, ServiceProbeSet, ServiceProbeSpec,
        ServiceRuntimeSpec, SERVICE_ARTIFACT_SCHEMA_VERSION,
    };

    const PROJECT: &str = "proj:mig";
    const APP: &str = "app:mig";

    fn dcontinuum_for(project_id: &str) -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            dcontinuum_id: format!("continuum:{project_id}"),
            revision: 1,
            nodes: ["node-a", "node-b", "node-c"]
                .into_iter()
                .map(|node_id| CanonicalDNodeV1 {
                    node_id: node_id.to_string(),
                    platform: CanonicalNodePlatformV1 {
                        os: "linux".to_string(),
                        architecture: "amd64".to_string(),
                    },
                    capacity: CanonicalNodeCapacityV1 {
                        cpu_cores: 4,
                        memory_bytes: 4_294_967_296,
                    },
                    capabilities: CanonicalNodeCapabilitiesV1 {
                        dnode_abi_versions: vec![DNODE_ABI_V0.to_string()],
                    },
                })
                .collect(),
            links: vec![],
        }
    }

    fn dgraph_for(application_id: &str) -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: application_id.to_string(),
            dgraph_id: format!("dgraph:{application_id}"),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_id: "svc".to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 134_217_728,
                },
            }],
            calls: vec![],
        }
    }

    fn artifact(node_id: &str) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: "artifact:svc".to_string(),
            service_id: "svc".to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                container: None,
                native_process: None,
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "none".to_string(),
                    params: Default::default(),
                    vantage_point: None,
                },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                target_nodes: vec![node_id.to_string()],
                ..Default::default()
            },
        }
    }

    fn placements(node_id: &str) -> Vec<CanonicalDServPlacementV1> {
        vec![CanonicalDServPlacementV1 {
            service_id: "svc".to_string(),
            node_id: node_id.to_string(),
        }]
    }

    #[allow(clippy::too_many_arguments)]
    fn create_pending_scoped(
        state: &mut DDeployStateV1,
        proposal_id: &str,
        project_id: &str,
        application_id: &str,
        node_id: &str,
        now: &str,
    ) {
        create_proposal(
            state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority: Default::default(),
                project_id: project_id.to_string(),
                application_id: application_id.to_string(),
                source: ProposalSource::Operator,
                dgraph: dgraph_for(application_id),
                dcontinuum: dcontinuum_for(project_id),
                placements: placements(node_id),
            },
            proposal_id.to_string(),
            now.to_string(),
            &[artifact(node_id)],
        )
        .unwrap();
    }

    fn create_pending(state: &mut DDeployStateV1, proposal_id: &str, node_id: &str, now: &str) {
        create_pending_scoped(state, proposal_id, PROJECT, APP, node_id, now);
    }

    fn accept(
        state: &mut DDeployStateV1,
        proposal_id: &str,
        node_id: &str,
        acceptance_id: &str,
        now: &str,
    ) -> AcceptOutcome {
        accept_proposal(
            state,
            proposal_id,
            "operator:test",
            None,
            &[artifact(node_id)],
            Some(&dcontinuum_for(PROJECT)),
            now,
            acceptance_id.to_string(),
        )
        .unwrap()
    }

    fn create_and_accept(state: &mut DDeployStateV1, proposal_id: &str, node_id: &str, acceptance_id: &str, now: &str) {
        create_pending(state, proposal_id, node_id, now);
        accept(state, proposal_id, node_id, acceptance_id, now);
    }

    #[allow(clippy::too_many_arguments)]
    fn create_and_accept_scoped(
        state: &mut DDeployStateV1,
        proposal_id: &str,
        project_id: &str,
        application_id: &str,
        node_id: &str,
        acceptance_id: &str,
        now: &str,
    ) {
        create_pending_scoped(state, proposal_id, project_id, application_id, node_id, now);
        accept_proposal(
            state,
            proposal_id,
            "operator:test",
            None,
            &[artifact(node_id)],
            Some(&dcontinuum_for(project_id)),
            now,
            acceptance_id.to_string(),
        )
        .unwrap();
    }

    fn intent_for(state: &DDeployStateV1, target_proposal_id: &str) -> MigrationIntentV1 {
        let view = ddeploy::active_placement_authority(state, PROJECT, APP)
            .unwrap()
            .expect("test fixture expects an active authority");
        let target_digest = state.proposals.get(target_proposal_id).unwrap().proposal_digest.clone();
        MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: view.dmap_id,
                revision: view.revision,
                digest: view.digest,
            },
            target_proposal_id: target_proposal_id.to_string(),
            target_proposal_digest: target_digest,
        }
    }

    // ---- T1 / ReadyForAcceptance ---------------------------------------- //
    #[test]
    fn t1_exact_a_plus_pending_exact_b_is_ready_for_acceptance() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(relation, MigrationAuthorityRelation::ReadyForAcceptance);
    }

    // ---- T2 / NoActiveSource --------------------------------------------- //
    #[test]
    fn t2_no_active_source_yields_no_active_source_relation() {
        let mut state = DDeployStateV1::empty();
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:00:00Z");
        let target_digest = state.proposals.get("p-b").unwrap().proposal_digest.clone();
        let intent = MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:whatever".to_string(),
                revision: 1,
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            },
            target_proposal_id: "p-b".to_string(),
            target_proposal_digest: target_digest,
        };
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(relation, MigrationAuthorityRelation::NoActiveSource);
    }

    // ---- T3/T4 / intent project/application mismatch --------------------- //
    #[test]
    fn t3_wrong_project_in_intent_is_target_scope_mismatch() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.project_id = "proj:other".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH);
    }

    #[test]
    fn t4_wrong_application_in_intent_is_target_scope_mismatch() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.application_id = "app:other".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH);
    }

    // ---- T5/T6/T7 / source id/revision/digest mismatch -> SourceStale ---- //
    #[test]
    fn t5_wrong_source_dmap_id_yields_source_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.source_dmap_ref.id = "dmap:not-real".to_string();
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(relation, MigrationAuthorityRelation::SourceStale { .. }));
    }

    #[test]
    fn t6_wrong_source_revision_yields_source_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.source_dmap_ref.revision = 999;
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(relation, MigrationAuthorityRelation::SourceStale { .. }));
    }

    #[test]
    fn t7_wrong_source_digest_yields_source_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.source_dmap_ref.digest =
            "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_string();
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(relation, MigrationAuthorityRelation::SourceStale { .. }));
    }

    // ---- T8/T9 / target lookup + digest integrity ------------------------ //
    #[test]
    fn t8_unknown_target_proposal_fails_not_found() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        let intent = MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:x".to_string(),
                revision: 1,
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            },
            target_proposal_id: "p-does-not-exist".to_string(),
            target_proposal_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_NOT_FOUND);
    }

    #[test]
    fn t9_target_proposal_digest_mismatch_fails() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.target_proposal_digest =
            "sha256:2222222222222222222222222222222222222222222222222222222222222222".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_DIGEST_MISMATCH);
    }

    // ---- T10/T11 / target belongs to another project/application --------- //
    #[test]
    fn t10_target_proposal_belongs_to_another_project() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending_scoped(&mut state, "p-other-proj", "proj:other", APP, "node-b", "2030-01-01T00:01:00Z");
        let target_digest = state.proposals.get("p-other-proj").unwrap().proposal_digest.clone();
        let view = ddeploy::active_placement_authority(&state, PROJECT, APP).unwrap().unwrap();
        let intent = MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: view.dmap_id,
                revision: view.revision,
                digest: view.digest,
            },
            target_proposal_id: "p-other-proj".to_string(),
            target_proposal_digest: target_digest,
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH);
    }

    #[test]
    fn t11_target_proposal_belongs_to_another_application() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending_scoped(&mut state, "p-other-app", PROJECT, "app:other", "node-b", "2030-01-01T00:01:00Z");
        let target_digest = state.proposals.get("p-other-app").unwrap().proposal_digest.clone();
        let view = ddeploy::active_placement_authority(&state, PROJECT, APP).unwrap().unwrap();
        let intent = MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: view.dmap_id,
                revision: view.revision,
                digest: view.digest,
            },
            target_proposal_id: "p-other-app".to_string(),
            target_proposal_digest: target_digest,
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH);
    }

    // ---- T12 / target's own expected_active_dmap_revision mismatch ------- //
    #[test]
    fn t12_target_expected_active_dmap_revision_mismatch() {
        let mut state = DDeployStateV1::empty();
        // B is created BEFORE A is ever accepted, so `create_proposal`
        // auto-captures its `expected_active_dmap_revision` as `None`.
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:00:00Z");
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(relation, MigrationAuthorityRelation::TargetInvalidForSource { .. }));
    }

    // ---- T13/T14 / target already accepted -------------------------------- //
    #[test]
    fn t13_exact_b_accepted_is_recognized_as_target_already_accepted() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        match relation {
            MigrationAuthorityRelation::TargetAlreadyAccepted { materialized_dmap_ref } => {
                let current = ddeploy::active_placement_authority(&state, PROJECT, APP).unwrap().unwrap();
                assert_eq!(materialized_dmap_ref.id, current.dmap_id);
                assert_eq!(materialized_dmap_ref.revision, current.revision);
                assert_eq!(materialized_dmap_ref.digest, current.digest);
            }
            other => panic!("expected TargetAlreadyAccepted, got {other:?}"),
        }
    }

    #[test]
    fn t14_crash_after_acceptance_recognizes_b_as_winner_even_though_a_is_stale() {
        // Simulates re-evaluating the SAME intent object as if the process
        // had crashed right after B's acceptance committed -- no workflow
        // state survives, only the intent (source A ref + target B ref) and
        // current D-Deploy state.
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b"); // captured while A was still current
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z"); // B wins
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(
            matches!(relation, MigrationAuthorityRelation::TargetAlreadyAccepted { .. }),
            "must recognize B as the winner, not misreport SourceStale, even though A (the captured source) is no longer current: {relation:?}"
        );
    }

    // ---- T15/T21 / unrelated proposal C wins -> SourceStale, never TargetAlreadyAccepted ---- //
    #[test]
    fn t15_unrelated_proposal_c_winning_is_source_stale_not_target_accepted() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        // An unrelated proposal C is created and accepted -- B is never
        // touched, but the active authority moves away from A anyway.
        create_and_accept(&mut state, "p-c", "node-c", "acc-c", "2030-01-01T00:02:00Z");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(
            matches!(relation, MigrationAuthorityRelation::SourceStale { .. }),
            "unrelated C's acceptance must be reported as SourceStale, never TargetAlreadyAccepted: {relation:?}"
        );
        assert_eq!(
            state.proposals.get("p-b").unwrap().state,
            ProposalLifecycleState::Pending,
            "B must remain untouched/Pending -- evaluating the relation never mutates it"
        );
    }

    #[test]
    fn t21_target_pending_but_source_changed_is_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        create_and_accept(&mut state, "p-c", "node-c", "acc-c", "2030-01-01T00:02:00Z");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(relation, MigrationAuthorityRelation::SourceStale { .. }));
    }

    // ---- T16 / idempotent re-evaluation ------------------------------------ //
    #[test]
    fn t16_repeat_evaluation_after_b_accepted_is_deterministic() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        let r1 = evaluate_migration_authority_relation(&state, &intent).unwrap();
        let r2 = evaluate_migration_authority_relation(&state, &intent).unwrap();
        let r3 = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(r1, r2);
        assert_eq!(r2, r3);
    }

    // ---- T23 / unrelated map entries/ordering cannot affect the result --- //
    #[test]
    fn t23_unrelated_state_content_cannot_affect_deterministic_result() {
        let mut state_a = DDeployStateV1::empty();
        create_and_accept(&mut state_a, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state_a, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent_a = intent_for(&state_a, "p-b");

        // Same semantics, but an unrelated, separately-scoped application's
        // own accepted proposal is inserted FIRST, before A/B exist at all.
        // Genuinely unrelated: a different application entirely, so it
        // shares no revision counter with proj:mig/app:mig.
        let mut state_b = DDeployStateV1::empty();
        create_and_accept_scoped(&mut state_b, "p-unrelated", "proj:other", "app:other", "node-c", "acc-unrelated", "2029-06-01T00:00:00Z");
        create_and_accept(&mut state_b, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state_b, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent_b = intent_for(&state_b, "p-b");

        assert_eq!(intent_a, intent_b);
        let relation_a = evaluate_migration_authority_relation(&state_a, &intent_a).unwrap();
        let relation_b = evaluate_migration_authority_relation(&state_b, &intent_b).unwrap();
        assert_eq!(relation_a, relation_b);
    }

    // ---- T24 / empty/invalid IDs fail structurally ------------------------ //
    #[test]
    fn t24_empty_project_id_fails_structurally() {
        let state = DDeployStateV1::empty();
        let intent = MigrationIntentV1 {
            project_id: "".to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:x".to_string(),
                revision: 1,
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            },
            target_proposal_id: "p-b".to_string(),
            target_proposal_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_INTENT_MALFORMED);
    }

    #[test]
    fn t24_malformed_digest_fails_structurally() {
        let state = DDeployStateV1::empty();
        let intent = MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:x".to_string(),
                revision: 1,
                digest: "not-a-digest".to_string(),
            },
            target_proposal_id: "p-b".to_string(),
            target_proposal_digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string(),
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_INTENT_MALFORMED);
    }

    // ---- T25 / never reads DMonitor ---------------------------------------- //
    #[test]
    fn t25_evaluation_touches_no_dmonitor_type() {
        // Structural proof, not a runtime one: `evaluate_migration_authority_relation`'s
        // only parameters are `&DDeployStateV1` and `&MigrationIntentV1` --
        // there is no DMonitor state type this function could read even if
        // it wanted to. Exercised here simply to keep a real, passing test
        // anchored to that claim.
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        assert!(evaluate_migration_authority_relation(&state, &intent).is_ok());
    }

    // ---- T26/T28/T29 / read-only, no D-Engine effect ----------------------- //
    #[test]
    fn t26_and_t28_pending_b_cannot_alter_dengine_membership_and_evaluation_is_read_only() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");

        let before = state.clone();
        let view_before = ddeploy::active_placement_authority(&state, PROJECT, APP).unwrap().unwrap();
        let dengine_before = ddeploy::derive_active_dengine_nodes(&view_before);

        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(relation, MigrationAuthorityRelation::ReadyForAcceptance);

        // No mutation occurred as a side effect of evaluation.
        assert_eq!(state, before);

        // D-Engine membership is still exactly A's -- a pending, fully
        // valid, ready-for-acceptance target B has no effect on it
        // whatsoever (Phase 118, unmodified and re-verified here).
        let view_after = ddeploy::active_placement_authority(&state, PROJECT, APP).unwrap().unwrap();
        let dengine_after = ddeploy::derive_active_dengine_nodes(&view_after);
        assert_eq!(dengine_before, dengine_after);
        assert_eq!(dengine_after, vec!["node-a".to_string()]);
    }
}
