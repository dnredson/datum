//! Phase 119B/119C: a small, pure, NON-AUTHORITATIVE core that relates an
//! exact source placement-authority reference to an exact target D-Deploy
//! proposal reference, and derives their CURRENT relationship from
//! already-existing, already-persisted D-Deploy state.
//!
//! This module owns nothing. It never selects a target, never mutates any
//! D-Deploy/D-Map/D-Node/D-Code/D-Monitor state, and never persists
//! anything of its own. It exists solely to answer, deterministically and
//! read-only: "given this exact source authority reference and this exact
//! target proposal reference, what is their relationship right now?" --
//! see `documentation/reference/phase119b-migration-intent-authority-relation-core.md`
//! and `documentation/reference/phase119c-migration-authority-currentness-hardening.md`
//! for the full audit trail behind every decision made here, in particular
//! why no persisted `MigrationRecord`/workflow state machine exists (Phase
//! 119A's own recommendation, refined by 119B's task framing: facts like
//! "content prepared"/"cleaning up" cannot be honestly derived from
//! canonical state alone, so this module never invents them).
//!
//! `MigrationIntentV1` is deliberately NOT a canonical `datum.*` wire
//! schema: it carries no `schema` field, is never submitted/validated/
//! accepted the way a D-Deploy proposal is, and is never persisted. Its
//! `Debug`/`Clone`/`PartialEq`/`Eq` derives exist purely for internal-caller
//! and test ergonomics -- it deliberately derives neither `Serialize` nor
//! `Deserialize`, exactly like `PlacementAuthorityView` itself (Phase
//! 117/118) carries no `schema` field and is never treated as wire
//! authority either.
//!
//! Phase 119C hardening: historical target acceptance is now distinguished
//! from CURRENT canonical authority (§119C decision -- see
//! `MigrationAuthorityRelation::TargetAcceptedAndCurrent` vs.
//! `TargetAcceptedButSuperseded`), accepted-target evidence is relationally
//! validated (not merely dereferenced) against its own claimed proposal
//! identity, and the same `target_proposal_id`/`acceptance_id` existing in
//! BOTH the v1 and v2 D-Deploy stores is treated as ambiguous persisted
//! identity and fails closed rather than silently preferring v1.

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
        x.len() == 64
            && x.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

pub const F_MIGRATION_INTENT_MALFORMED: &str = "migration_intent_malformed";
pub const F_MIGRATION_TARGET_PROPOSAL_NOT_FOUND: &str = "migration_target_proposal_not_found";
pub const F_MIGRATION_TARGET_PROPOSAL_AMBIGUOUS: &str = "migration_target_proposal_ambiguous";
pub const F_MIGRATION_TARGET_PROPOSAL_DIGEST_CORRUPTED: &str =
    "migration_target_proposal_digest_corrupted";
pub const F_MIGRATION_TARGET_PROPOSAL_DIGEST_MISMATCH: &str =
    "migration_target_proposal_digest_mismatch";
pub const F_MIGRATION_TARGET_PROPOSAL_SCOPE_MISMATCH: &str =
    "migration_target_proposal_scope_mismatch";
pub const F_MIGRATION_TARGET_ACCEPTANCE_MISSING: &str = "migration_target_acceptance_missing";
pub const F_MIGRATION_TARGET_ACCEPTANCE_AMBIGUOUS: &str = "migration_target_acceptance_ambiguous";
pub const F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT: &str =
    "migration_target_acceptance_inconsistent";
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
/// doc, the 119B phase document's "what 119B cannot know" section, and the
/// 119C phase document §4/§5).
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
    /// `TargetAcceptedAndCurrent`/`TargetAcceptedButSuperseded`, which are
    /// always checked first and take priority). Some other, unrelated
    /// acceptance won since the intent's source reference was captured.
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
    /// was accepted, its acceptance evidence relationally validates against
    /// that exact proposal (Phase 119C §8), AND the D-Map it materialized
    /// is STILL the current active authority right now. This is the answer
    /// a crash-immediately-after-acceptance re-evaluation must reach, and
    /// the answer that correctly distinguishes "this exact migration's
    /// target won and remains current" from "some unrelated proposal won"
    /// (`SourceStale`) or "this target won but was since superseded"
    /// (`TargetAcceptedButSuperseded`). This is a legitimate, expected
    /// relation, never an error. It means ONLY that canonical placement
    /// cutover occurred and remains current -- it does NOT mean bytes were
    /// installed, a runtime started, the target is healthy, source cleanup
    /// completed, or the application is operationally converged (Phase
    /// 119A §32; Phase 119C §4), none of which this module can or does
    /// claim to know.
    TargetAcceptedAndCurrent {
        materialized_dmap_ref: CanonicalContractReferenceV1,
    },
    /// The exact target proposal was accepted and its acceptance evidence
    /// relationally validates (Phase 119C §8) -- that historical fact is
    /// permanent and immutable -- but the D-Map it materialized is NO
    /// LONGER the current active authority: a later acceptance (for the
    /// same application) has since superseded it. `current_authority` is
    /// the live replacement, or `None` in the (not known to be reachable
    /// through any existing legitimate D-Deploy path, since acceptances are
    /// never removed/unaccepted -- handled defensively rather than assumed
    /// impossible) case where no active authority currently exists at all.
    /// Like `TargetAcceptedAndCurrent`, this says nothing about runtime
    /// preparation, health, cleanup, or convergence.
    TargetAcceptedButSuperseded {
        materialized_dmap_ref: CanonicalContractReferenceV1,
        current_authority: Option<CanonicalContractReferenceV1>,
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

/// A normalized, v1/v2-neutral view of exactly the facts this module needs
/// from an acceptance record, so accepted-target evidence can be
/// relationally validated (Phase 119C §8) rather than merely dereferenced.
struct NormalizedAcceptance {
    proposal_id: String,
    proposal_digest: String,
    project_id: String,
    application_id: String,
    materialized_dmap_ref: CanonicalContractReferenceV1,
}

/// `Err(())` iff `proposal_id` exists in BOTH the v1 and v2 D-Deploy
/// proposal stores -- ambiguous persisted identity, treated as fail-closed
/// (Phase 119C §9) rather than silently preferring v1. Normal production
/// proposal IDs are server/caller-generated (typically UUID-shaped) and
/// nothing in `create_proposal`/`DDeployStateV2::create_proposal` checks
/// the OTHER store before inserting, so no existing repository invariant
/// proves this collision impossible -- this function does not assume it is.
fn locate_target_proposal(
    state: &DDeployStateV1,
    proposal_id: &str,
) -> Result<Option<NormalizedTargetProposal>, ()> {
    let v1 = state.proposals.get(proposal_id);
    let v2 = state.phase113_v2.proposals.get(proposal_id);
    match (v1, v2) {
        (Some(_), Some(_)) => Err(()),
        (Some(record), None) => Ok(Some(NormalizedTargetProposal {
            project_id: record.proposal.project_id.clone(),
            application_id: record.proposal.application_id.clone(),
            stored_digest: record.proposal_digest.clone(),
            recomputed_digest: ddeploy::compute_proposal_digest(&record.proposal),
            expected_active_dmap_revision: record.proposal.expected_active_dmap_revision,
            accepted: record.state == ProposalLifecycleState::Accepted,
            accepted_acceptance_id: record.accepted_acceptance_id.clone(),
        })),
        (None, Some(record)) => Ok(Some(NormalizedTargetProposal {
            project_id: record.proposal.project_id.clone(),
            application_id: record.proposal.application_id.clone(),
            stored_digest: record.proposal_digest.clone(),
            recomputed_digest: ddeploy_v2::DDeployStateV2::proposal_digest(&record.proposal),
            expected_active_dmap_revision: record.proposal.expected_active_dmap_revision,
            accepted: record.state == ProposalV2State::Accepted,
            accepted_acceptance_id: record.accepted_acceptance_id.clone(),
        })),
        (None, None) => Ok(None),
    }
}

/// Same ambiguity-detection discipline as `locate_target_proposal`, applied
/// to acceptance identity (Phase 119C §9/§10).
fn locate_acceptance(
    state: &DDeployStateV1,
    acceptance_id: &str,
) -> Result<Option<NormalizedAcceptance>, ()> {
    let v1 = state.acceptances.get(acceptance_id);
    let v2 = state.phase113_v2.acceptances.get(acceptance_id);
    match (v1, v2) {
        (Some(_), Some(_)) => Err(()),
        (Some(a), None) => Ok(Some(NormalizedAcceptance {
            proposal_id: a.proposal_id.clone(),
            proposal_digest: a.proposal_digest.clone(),
            project_id: a.project_id.clone(),
            application_id: a.application_id.clone(),
            materialized_dmap_ref: a.materialized_dmap_ref.clone(),
        })),
        (None, Some(a)) => Ok(Some(NormalizedAcceptance {
            proposal_id: a.proposal_id.clone(),
            proposal_digest: a.proposal_digest.clone(),
            project_id: a.project_id.clone(),
            application_id: a.application_id.clone(),
            materialized_dmap_ref: CanonicalContractReferenceV1 {
                id: a.materialized_dmap_ref.id.clone(),
                revision: a.materialized_dmap_ref.revision,
                digest: a.materialized_dmap_ref.digest.clone(),
            },
        })),
        (None, None) => Ok(None),
    }
}

/// Wraps `active_placement_authority`, mapping its own `Err` (corrupted
/// canonical authority -- e.g. dual active v1/v2 authority, or an active
/// record referencing a missing proposal) to this module's fail-closed
/// finding. Shared by both the pending-target path and the accepted-target
/// currentness check (Phase 119C §7) so corrupted canonical authority is
/// never downgraded into a legitimate relation like
/// `TargetAcceptedButSuperseded`.
fn current_source_ref(
    state: &DDeployStateV1,
    project_id: &str,
    application_id: &str,
) -> Result<Option<CanonicalContractReferenceV1>, CanonicalValidationFinding> {
    match ddeploy::active_placement_authority(state, project_id, application_id) {
        Err(error) => Err(finding(F_MIGRATION_SOURCE_AUTHORITY_CORRUPTED, error)),
        Ok(None) => Ok(None),
        Ok(Some(view)) => Ok(Some(CanonicalContractReferenceV1 {
            id: view.dmap_id,
            revision: view.revision,
            digest: view.digest,
        })),
    }
}

fn validate_intent_shape(intent: &MigrationIntentV1) -> Result<(), CanonicalValidationFinding> {
    if intent.project_id.trim().is_empty() {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "project_id must be non-empty",
        ));
    }
    if intent.application_id.trim().is_empty() {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "application_id must be non-empty",
        ));
    }
    if intent.source_dmap_ref.id.trim().is_empty() {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "source_dmap_ref.id must be non-empty",
        ));
    }
    // Phase 119C §10: mirrors the existing canonical D-Map contract
    // (`CanonicalDMapV1`/`CanonicalDMapV2` both reject `revision < 1`, and
    // `accept_proposal`/`accept_with_previous` always produce revision >= 1)
    // rather than inventing an incompatible rule.
    if intent.source_dmap_ref.revision < 1 {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "source_dmap_ref.revision must be >= 1, matching the canonical D-Map contract",
        ));
    }
    if !digest_ok(&intent.source_dmap_ref.digest) {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "source_dmap_ref.digest must be a canonical 'sha256:<64 lowercase hex>' digest",
        ));
    }
    if intent.target_proposal_id.trim().is_empty() {
        return Err(finding(
            F_MIGRATION_INTENT_MALFORMED,
            "target_proposal_id must be non-empty",
        ));
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
/// `documentation/reference/phase119b-migration-intent-authority-relation-core.md` /
/// `documentation/reference/phase119c-migration-authority-currentness-hardening.md`
/// for the full decision-tree rationale.
pub fn evaluate_migration_authority_relation(
    state: &DDeployStateV1,
    intent: &MigrationIntentV1,
) -> Result<MigrationAuthorityRelation, CanonicalValidationFinding> {
    validate_intent_shape(intent)?;

    let target = match locate_target_proposal(state, &intent.target_proposal_id) {
        Err(()) => {
            return Err(finding(
                F_MIGRATION_TARGET_PROPOSAL_AMBIGUOUS,
                format!(
                    "target proposal_id '{}' exists in BOTH the v1 and v2 D-Deploy proposal stores -- ambiguous persisted identity",
                    intent.target_proposal_id
                ),
            ));
        }
        Ok(None) => {
            return Err(finding(
                F_MIGRATION_TARGET_PROPOSAL_NOT_FOUND,
                format!(
                    "target proposal '{}' does not exist in the supplied D-Deploy state",
                    intent.target_proposal_id
                ),
            ));
        }
        Ok(Some(target)) => target,
    };

    let recomputed = target.recomputed_digest.as_deref().map_err(|error| {
        finding(
            F_MIGRATION_TARGET_PROPOSAL_DIGEST_CORRUPTED,
            format!(
                "target proposal '{}' digest could not be recomputed: {error}",
                intent.target_proposal_id
            ),
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
    // "unrelated proposal won" distinction correct (Phase 119A §28; Phase
    // 119B/119C's own crash-recovery requirement). Whatever the current
    // active authority is right now, if THIS exact target proposal was
    // accepted, that fact is independently, permanently true and must be
    // reported as such rather than mis-derived as "the source went stale."
    if target.accepted {
        let acceptance_id = target.accepted_acceptance_id.as_deref().ok_or_else(|| {
            finding(
                F_MIGRATION_TARGET_ACCEPTANCE_MISSING,
                format!(
                    "target proposal '{}' is marked Accepted but carries no accepted_acceptance_id",
                    intent.target_proposal_id
                ),
            )
        })?;
        let acceptance = match locate_acceptance(state, acceptance_id) {
            Err(()) => {
                return Err(finding(
                    F_MIGRATION_TARGET_ACCEPTANCE_AMBIGUOUS,
                    format!(
                        "acceptance_id '{acceptance_id}' referenced by target proposal '{}' exists in BOTH the v1 and v2 D-Deploy acceptance stores -- ambiguous persisted identity",
                        intent.target_proposal_id
                    ),
                ));
            }
            Ok(None) => {
                return Err(finding(
                    F_MIGRATION_TARGET_ACCEPTANCE_MISSING,
                    format!("target proposal '{}' references acceptance '{acceptance_id}', which does not exist in the supplied D-Deploy state", intent.target_proposal_id),
                ));
            }
            Ok(Some(acceptance)) => acceptance,
        };

        // Phase 119C §8: relational integrity, not merely a pointer
        // dereference -- the acceptance record must actually claim to
        // belong to THIS exact target proposal, in THIS exact scope,
        // before its `materialized_dmap_ref` is trusted for anything.
        if acceptance.proposal_id != intent.target_proposal_id
            || acceptance.proposal_digest != target.stored_digest
            || acceptance.project_id != intent.project_id
            || acceptance.application_id != intent.application_id
        {
            return Err(finding(
                F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT,
                format!(
                    "acceptance '{acceptance_id}' does not relationally match target proposal '{}': acceptance claims proposal_id='{}' proposal_digest='{}' project_id='{}' application_id='{}'",
                    intent.target_proposal_id, acceptance.proposal_id, acceptance.proposal_digest, acceptance.project_id, acceptance.application_id
                ),
            ));
        }

        let materialized_dmap_ref = acceptance.materialized_dmap_ref;
        return match current_source_ref(state, &intent.project_id, &intent.application_id)? {
            Some(current) if current == materialized_dmap_ref => {
                Ok(MigrationAuthorityRelation::TargetAcceptedAndCurrent {
                    materialized_dmap_ref,
                })
            }
            current_authority => Ok(MigrationAuthorityRelation::TargetAcceptedButSuperseded {
                materialized_dmap_ref,
                current_authority,
            }),
        };
    }

    match current_source_ref(state, &intent.project_id, &intent.application_id)? {
        None => Ok(MigrationAuthorityRelation::NoActiveSource),
        Some(current_source) => {
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
    use crate::core::canonical_dforward::{CanonicalDForwardV1, CANONICAL_DFORWARD_SCHEMA};
    use crate::core::canonical_dgraph::{
        CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
        CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA,
        DNODE_ABI_V0,
    };
    use crate::core::canonical_dmap::CanonicalDServPlacementV1;
    use crate::core::ddeploy::{
        accept_proposal, create_proposal, AcceptOutcome, CreateProposalInput, ProposalSource,
        DDEPLOY_ACCEPTANCE_SCHEMA,
    };
    use crate::core::ddeploy_v2::{
        DDeployAcceptanceV2, DDeployProposalV2, DDeployProposalV2Record, ProposalV2State,
    };
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
                service_definition: None, service_id: "svc".to_string(),
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

    fn create_and_accept(
        state: &mut DDeployStateV1,
        proposal_id: &str,
        node_id: &str,
        acceptance_id: &str,
        now: &str,
    ) {
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
        let target_digest = state
            .proposals
            .get(target_proposal_id)
            .unwrap()
            .proposal_digest
            .clone();
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

    /// A minimal, structurally-complete (but NOT necessarily semantically
    /// validated) `DDeployProposalV2Record`, inserted DIRECTLY into
    /// `state.phase113_v2.proposals` -- bypassing `DDeployStateV2::create_proposal`
    /// entirely -- purely so tests can construct a v1/v2 `proposal_id`
    /// collision to prove `locate_target_proposal` fails closed on it
    /// (Phase 119C §9). Never a template for legitimate v2 proposal
    /// construction.
    fn insert_raw_v2_proposal_stub(state: &mut DDeployStateV1, proposal_id: &str) {
        let dgraph = dgraph_for("app:stub");
        let dcontinuum = dcontinuum_for("proj:stub");
        let dforward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: "proj:stub".to_string(),
            dforward_id: "dforward:stub".to_string(),
            revision: 1,
            diots: vec![],
            external_endpoints: vec![],
            chains: vec![],
            support_jobs: vec![],
        };
        let contract_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: "stub".to_string(),
            revision: 1,
            digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_string(),
        };
        let proposal = DDeployProposalV2 {
            schema: "datum.ddeploy-proposal/2".to_string(),
            proposal_id: proposal_id.to_string(),
            project_id: "proj:stub".to_string(),
            application_id: "app:stub".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2030-01-01T00:00:00Z".to_string(),
            dgraph_ref: contract_ref.clone(),
            dcontinuum_ref: contract_ref.clone(),
            dforward_ref: contract_ref,
            placements: vec![],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: None,
            diot_artifact_authority: Default::default(),
            dcode_artifact_authority: Default::default(),
        };
        // Ambiguity is detected (and this record's own digest is never
        // read) before any digest comparison runs -- see
        // `locate_target_proposal` -- so an approximate digest is
        // sufficient here.
        let proposal_digest = ddeploy_v2::DDeployStateV2::proposal_digest(&proposal)
            .unwrap_or_else(|_| {
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string()
            });
        state.phase113_v2.proposals.insert(
            proposal_id.to_string(),
            DDeployProposalV2Record {
                proposal_digest,
                proposal,
                dgraph_snapshot: dgraph,
                dcontinuum_snapshot: dcontinuum,
                dforward_snapshot: dforward,
                state: ProposalV2State::Pending,
                accepted_acceptance_id: None,
            },
        );
    }

    /// A minimal, structurally-complete `DDeployAcceptanceV2`, inserted
    /// DIRECTLY into `state.phase113_v2.acceptances` for the same reason as
    /// `insert_raw_v2_proposal_stub` -- proving `locate_acceptance` fails
    /// closed on a v1/v2 `acceptance_id` collision (Phase 119C §9/§10).
    fn insert_raw_v2_acceptance_stub(state: &mut DDeployStateV1, acceptance_id: &str) {
        state.phase113_v2.acceptances.insert(
            acceptance_id.to_string(),
            DDeployAcceptanceV2 {
                schema: DDEPLOY_ACCEPTANCE_SCHEMA.to_string(),
                acceptance_id: acceptance_id.to_string(),
                proposal_id: "stub-proposal".to_string(),
                proposal_digest:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                project_id: "proj:stub".to_string(),
                application_id: "app:stub".to_string(),
                accepted_at_utc: "2030-01-01T00:00:00Z".to_string(),
                accepted_by: "test-stub".to_string(),
                materialized_dmap_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                    id: "dmap:stub".to_string(),
                    revision: 1,
                    digest:
                        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                            .to_string(),
                },
                projection_ref: None,
            },
        );
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
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
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
        assert!(matches!(
            relation,
            MigrationAuthorityRelation::SourceStale { .. }
        ));
    }

    #[test]
    fn t6_wrong_source_revision_yields_source_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let mut intent = intent_for(&state, "p-b");
        intent.source_dmap_ref.revision = 999;
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(
            relation,
            MigrationAuthorityRelation::SourceStale { .. }
        ));
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
        assert!(matches!(
            relation,
            MigrationAuthorityRelation::SourceStale { .. }
        ));
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
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            },
            target_proposal_id: "p-does-not-exist".to_string(),
            target_proposal_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
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
        create_pending_scoped(
            &mut state,
            "p-other-proj",
            "proj:other",
            APP,
            "node-b",
            "2030-01-01T00:01:00Z",
        );
        let target_digest = state
            .proposals
            .get("p-other-proj")
            .unwrap()
            .proposal_digest
            .clone();
        let view = ddeploy::active_placement_authority(&state, PROJECT, APP)
            .unwrap()
            .unwrap();
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
        create_pending_scoped(
            &mut state,
            "p-other-app",
            PROJECT,
            "app:other",
            "node-b",
            "2030-01-01T00:01:00Z",
        );
        let target_digest = state
            .proposals
            .get("p-other-app")
            .unwrap()
            .proposal_digest
            .clone();
        let view = ddeploy::active_placement_authority(&state, PROJECT, APP)
            .unwrap()
            .unwrap();
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
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:00:00Z");
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(matches!(
            relation,
            MigrationAuthorityRelation::TargetInvalidForSource { .. }
        ));
    }

    // ---- H1/H2 / accepted and current, incl. crash-after-acceptance ------ //
    #[test]
    fn h1_exact_b_accepted_and_current_is_target_accepted_and_current() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        match relation {
            MigrationAuthorityRelation::TargetAcceptedAndCurrent {
                materialized_dmap_ref,
            } => {
                let current = ddeploy::active_placement_authority(&state, PROJECT, APP)
                    .unwrap()
                    .unwrap();
                assert_eq!(materialized_dmap_ref.id, current.dmap_id);
                assert_eq!(materialized_dmap_ref.revision, current.revision);
                assert_eq!(materialized_dmap_ref.digest, current.digest);
            }
            other => panic!("expected TargetAcceptedAndCurrent, got {other:?}"),
        }
    }

    #[test]
    fn h2_crash_after_acceptance_recognizes_b_as_accepted_and_current() {
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
            matches!(relation, MigrationAuthorityRelation::TargetAcceptedAndCurrent { .. }),
            "must recognize B as accepted AND current, not misreport SourceStale, even though A (the captured source) is no longer current: {relation:?}"
        );
    }

    // ---- H3/H4 / accepted but superseded, and deterministic re-evaluation - //
    #[test]
    fn h3_b_accepted_then_c_accepted_is_target_accepted_but_superseded() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z"); // B wins
        create_and_accept(&mut state, "p-c", "node-c", "acc-c", "2030-01-01T00:03:00Z"); // C supersedes B

        let b_materialized = match evaluate_migration_authority_relation(&state, &intent).unwrap() {
            MigrationAuthorityRelation::TargetAcceptedButSuperseded {
                materialized_dmap_ref,
                current_authority,
            } => {
                let current = ddeploy::active_placement_authority(&state, PROJECT, APP)
                    .unwrap()
                    .unwrap();
                let current_ref = CanonicalContractReferenceV1 {
                    id: current.dmap_id,
                    revision: current.revision,
                    digest: current.digest,
                };
                assert_eq!(current_authority, Some(current_ref.clone()));
                assert_ne!(
                    materialized_dmap_ref, current_ref,
                    "B's materialization must differ from C's, or this test proves nothing"
                );
                materialized_dmap_ref
            }
            other => panic!("expected TargetAcceptedButSuperseded, got {other:?}"),
        };
        assert_eq!(
            b_materialized.revision, 2,
            "B was the second acceptance for this application"
        );
    }

    #[test]
    fn h4_repeated_evaluation_of_superseded_target_is_deterministic() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        create_and_accept(&mut state, "p-c", "node-c", "acc-c", "2030-01-01T00:03:00Z");
        let r1 = evaluate_migration_authority_relation(&state, &intent).unwrap();
        let r2 = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(r1, r2);
        assert!(matches!(
            r1,
            MigrationAuthorityRelation::TargetAcceptedButSuperseded { .. }
        ));
    }

    // ---- H5 / unrelated C before pending B remains SourceStale ----------- //
    #[test]
    fn h5_unrelated_c_accepted_before_pending_b_is_still_source_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        create_and_accept(&mut state, "p-c", "node-c", "acc-c", "2030-01-01T00:02:00Z");
        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert!(
            matches!(relation, MigrationAuthorityRelation::SourceStale { .. }),
            "unrelated C's acceptance must be reported as SourceStale, never as any accepted-target relation: {relation:?}"
        );
        assert_eq!(
            state.proposals.get("p-b").unwrap().state,
            ProposalLifecycleState::Pending,
            "B must remain untouched/Pending -- evaluating the relation never mutates it"
        );
    }

    // ---- H6-H10 / acceptance relational-integrity hardening -------------- //
    #[test]
    fn h6_accepted_proposal_pointing_to_nonexistent_acceptance_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        // Corrupt the persisted state: remove the acceptance record the
        // proposal's own `accepted_acceptance_id` still points to.
        state.acceptances.remove("acc-b");
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_ACCEPTANCE_MISSING);
    }

    #[test]
    fn h7_accepted_proposal_acceptance_for_wrong_proposal_id_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        state.acceptances.get_mut("acc-b").unwrap().proposal_id = "p-not-b".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT);
    }

    #[test]
    fn h8_accepted_proposal_acceptance_digest_mismatch_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        state.acceptances.get_mut("acc-b").unwrap().proposal_digest =
            "sha256:3333333333333333333333333333333333333333333333333333333333333333".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT);
    }

    #[test]
    fn h9_accepted_proposal_acceptance_project_mismatch_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        state.acceptances.get_mut("acc-b").unwrap().project_id = "proj:tampered".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT);
    }

    #[test]
    fn h10_accepted_proposal_acceptance_application_mismatch_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        state.acceptances.get_mut("acc-b").unwrap().application_id = "app:tampered".to_string();
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT);
    }

    // ---- H11/H12 / v1-v2 identity ambiguity ------------------------------- //
    #[test]
    fn h11_same_target_proposal_id_in_both_v1_and_v2_stores_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        insert_raw_v2_proposal_stub(&mut state, "p-b");
        let intent = intent_for(&state, "p-b");
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_PROPOSAL_AMBIGUOUS);
    }

    #[test]
    fn h12_same_acceptance_id_in_both_v1_and_v2_stores_fails_closed() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        insert_raw_v2_acceptance_stub(&mut state, "acc-b");
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_TARGET_ACCEPTANCE_AMBIGUOUS);
    }

    // ---- H13 / source revision zero rejected ------------------------------ //
    #[test]
    fn h13_source_revision_zero_is_rejected_as_malformed() {
        let state = DDeployStateV1::empty();
        let intent = MigrationIntentV1 {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source_dmap_ref: CanonicalContractReferenceV1 {
                id: "dmap:x".to_string(),
                revision: 0,
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            },
            target_proposal_id: "p-b".to_string(),
            target_proposal_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_INTENT_MALFORMED);
    }

    // ---- H17 / accepted B historical while current C is a v2 generation -- //
    #[test]
    fn h17_accepted_b_historical_while_current_authority_is_a_different_dmap_generation() {
        // B (v1) is accepted, then C is accepted as a v2 proposal for the
        // SAME application -- Phase 119A §49/119B §8 confirm this v1->v2
        // transition is already legal DDeploy behavior
        // (`DDeployStateV2::accept_with_previous`). The migration relation
        // must still correctly report B as superseded by C, regardless of
        // C's D-Map generation.
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");

        let previous = {
            let active = state.active_for(PROJECT, APP).unwrap();
            (
                active.active_dmap.dmap_id.clone(),
                active.active_dmap.revision,
            )
        };
        let dgraph = dgraph_for(APP);
        let dcontinuum = dcontinuum_for(PROJECT);
        let dforward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dforward_id: format!("dforward:{APP}"),
            revision: 1,
            diots: vec![],
            external_endpoints: vec![],
            chains: vec![],
            support_jobs: vec![],
        };
        let contract_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dgraph.dgraph_id.clone(),
            revision: dgraph.revision,
            digest: crate::core::canonical_dgraph::recompute_dgraph_digest(&dgraph).unwrap(),
        };
        let dcontinuum_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dcontinuum.dcontinuum_id.clone(),
            revision: dcontinuum.revision,
            digest: crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum)
                .unwrap(),
        };
        let dforward_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dforward.dforward_id.clone(),
            revision: dforward.revision,
            digest: dforward.digest().unwrap(),
        };
        let v2_proposal = DDeployProposalV2 {
            schema: "datum.ddeploy-proposal/2".to_string(),
            proposal_id: "p-c-v2".to_string(),
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2030-01-01T00:03:00Z".to_string(),
            dgraph_ref: contract_ref,
            dcontinuum_ref,
            dforward_ref,
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: "svc".to_string(),
                node_id: "node-c".to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: Some(previous.1),
            diot_artifact_authority: Default::default(),
            dcode_artifact_authority: Default::default(),
        };
        let dcontinuum_for_accept = dcontinuum_for(PROJECT);
        state
            .create_v2_proposal(v2_proposal, dgraph, dcontinuum, dforward)
            .unwrap();
        // The real production entry point (`DDeployStateV1::accept_v2_proposal`,
        // not the inner `DDeployStateV2::accept_with_previous` alone) --
        // this is the one that also removes the stale v1 active record as
        // part of the same transition, exactly as a real v1->v2 cutover
        // requires.
        state
            .accept_v2_proposal(
                "p-c-v2",
                "acc-c-v2".to_string(),
                "operator:test".to_string(),
                "2030-01-01T00:03:00Z".to_string(),
                &dcontinuum_for_accept,
                &[artifact("node-c")],
                None,
            )
            .unwrap();

        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        match relation {
            MigrationAuthorityRelation::TargetAcceptedButSuperseded { current_authority, .. } => {
                let current = ddeploy::active_placement_authority(&state, PROJECT, APP).unwrap().unwrap();
                assert_eq!(current.revision, 3, "sanity: p-a=1, p-b=2, p-c-v2=3");
                assert_eq!(
                    current_authority,
                    Some(CanonicalContractReferenceV1 { id: current.dmap_id, revision: current.revision, digest: current.digest })
                );
            }
            other => panic!("expected TargetAcceptedButSuperseded across a v1->v2 generation change, got {other:?}"),
        }
    }

    // ---- H18/H19 / read-only, no D-Engine effect --------------------------- //
    #[test]
    fn h18_and_h19_pending_b_cannot_alter_dengine_membership_and_evaluation_is_read_only() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");

        let before = state.clone();
        let view_before = ddeploy::active_placement_authority(&state, PROJECT, APP)
            .unwrap()
            .unwrap();
        let dengine_before = ddeploy::derive_active_dengine_nodes(&view_before);

        let relation = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(relation, MigrationAuthorityRelation::ReadyForAcceptance);

        // No mutation occurred as a side effect of evaluation.
        assert_eq!(state, before);

        // D-Engine membership is still exactly A's -- a pending, fully
        // valid, ready-for-acceptance target B has no effect on it
        // whatsoever (Phase 118, unmodified and re-verified here).
        let view_after = ddeploy::active_placement_authority(&state, PROJECT, APP)
            .unwrap()
            .unwrap();
        let dengine_after = ddeploy::derive_active_dengine_nodes(&view_after);
        assert_eq!(dengine_before, dengine_after);
        assert_eq!(dengine_after, vec!["node-a".to_string()]);
    }

    /// Same read-only guarantee, exercised on the NEW accepted-target
    /// currentness path (which now additionally calls
    /// `active_placement_authority` a second time internally) -- Phase 119C
    /// must not introduce any mutation there either.
    #[test]
    fn read_only_holds_on_the_accepted_and_current_path_too() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "p-a", "node-a", "acc-a", "2030-01-01T00:00:00Z");
        create_pending(&mut state, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent = intent_for(&state, "p-b");
        accept(&mut state, "p-b", "node-b", "acc-b", "2030-01-01T00:02:00Z");
        let before = state.clone();
        let _ = evaluate_migration_authority_relation(&state, &intent).unwrap();
        assert_eq!(state, before);
    }

    // ---- H20 / no DMonitor/runtime dependency ------------------------------ //
    #[test]
    fn h20_evaluation_touches_no_dmonitor_type() {
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

    // ---- T23 / unrelated map entries/ordering cannot affect the result --- //
    #[test]
    fn t23_unrelated_state_content_cannot_affect_deterministic_result() {
        let mut state_a = DDeployStateV1::empty();
        create_and_accept(
            &mut state_a,
            "p-a",
            "node-a",
            "acc-a",
            "2030-01-01T00:00:00Z",
        );
        create_pending(&mut state_a, "p-b", "node-b", "2030-01-01T00:01:00Z");
        let intent_a = intent_for(&state_a, "p-b");

        let mut state_b = DDeployStateV1::empty();
        create_and_accept_scoped(
            &mut state_b,
            "p-unrelated",
            "proj:other",
            "app:other",
            "node-c",
            "acc-unrelated",
            "2029-06-01T00:00:00Z",
        );
        create_and_accept(
            &mut state_b,
            "p-a",
            "node-a",
            "acc-a",
            "2030-01-01T00:00:00Z",
        );
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
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            },
            target_proposal_id: "p-b".to_string(),
            target_proposal_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
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
            target_proposal_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
        };
        let err = evaluate_migration_authority_relation(&state, &intent).unwrap_err();
        assert_eq!(err.code, F_MIGRATION_INTENT_MALFORMED);
    }
}
