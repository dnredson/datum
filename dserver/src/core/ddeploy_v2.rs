//! Explicit DDeploy/2 governance and the shared active-placement view.
use super::{
    canonical_dforward::CanonicalDForwardV1,
    canonical_dgraph::CanonicalDGraphV1,
    canonical_dmap_v2::{
        CanonicalDMapV2, ContractRefV2, DCallRealizationV2, DIoTPlacementV2, DServPlacementV2,
        ExternalPortBindingV2,
    },
    ddeploy::ProposalSource,
    jcs,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
pub const DDEPLOY_PROPOSAL_V2_SCHEMA: &str = "datum.ddeploy-proposal/2";
pub const DDEPLOY_ACCEPTANCE_V2_SCHEMA: &str = "datum.ddeploy-acceptance/2";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DDeployProposalV2 {
    pub schema: String,
    pub proposal_id: String,
    pub project_id: String,
    pub application_id: String,
    /// Phase 113A closure pass 4.1: provenance only -- WHO/WHAT produced
    /// this proposal. Reuses v1's exact `ProposalSource` type/semantics
    /// rather than a second enum. Never selects placement itself and never
    /// bypasses explicit acceptance: `source == Ai` still requires the exact
    /// same `accept_v2_proposal` call as any other source (see
    /// `ai_sourced_v2_proposal_has_no_alternate_activation_path`). Part of
    /// `DDeployProposalV2` itself (not the wrapping record), so it is
    /// automatically covered by `proposal_digest()` below -- changing it
    /// after creation without recomputing the digest is caught by
    /// `DDeployStateV2::validate_integrity()` on reload, exactly like every
    /// other field here.
    pub source: ProposalSource,
    /// Phase 113A closure pass 4.1: server-generated at creation time
    /// (`post_create_proposal_v2` calls `Utc::now()` itself, mirroring v1's
    /// `create_proposal` exactly -- never caller-suppliable), required to be
    /// RFC3339-valid by `schema_valid()` below, and digest-covered like
    /// `source`.
    pub created_at_utc: String,
    pub dgraph_ref: ContractRefV2,
    pub dcontinuum_ref: ContractRefV2,
    pub dforward_ref: ContractRefV2,
    pub placements: Vec<DServPlacementV2>,
    pub diot_placements: Vec<DIoTPlacementV2>,
    pub dcall_realizations: Vec<DCallRealizationV2>,
    pub external_port_bindings: Vec<ExternalPortBindingV2>,
    #[serde(default)]
    pub expected_active_dmap_revision: Option<u64>,
    /// Phase 113A closure pass 3: server-owned DIoT artifact authority,
    /// keyed by `artifact_id` (never by `diot_id` -- two DIoTs requiring the
    /// same artifact must resolve to the exact same binding, see
    /// `ddeploy::resolve_diot_artifact_authority`). Deliberately part of
    /// `DDeployProposalV2` itself, not `DDeployProposalV2Record`, so it is
    /// automatically load-bearing in `proposal_digest()` -- changing a
    /// bound artifact's `execution_generation`/`execution_projection_digest`
    /// changes the proposal digest. `BTreeMap` keying also makes the digest
    /// deterministic regardless of `CanonicalDForwardV1.diots`'s own order.
    #[serde(default)]
    pub diot_artifact_authority: BTreeMap<String, DIoTArtifactAuthorityBindingV2>,
}

/// Phase 113A closure pass 3: the SERVER-OWNED authority binding a DIoT
/// artifact requirement resolves to, restricted to fields the
/// `ServiceArtifactRegistry` genuinely and verifiably owns and recomputes on
/// every mutation (see `service_artifact_registry.rs`'s own doc comments on
/// `execution_generation`/`execution_projection_digest`). This is a
/// deliberately separate, OPERATIONAL concept from `DIoTArtifactBindingV2`
/// (the pre-existing, canonical-`datum.dmap/2` field carrying a
/// caller-declared `descriptor_digest`/`content_digest`, format-checked only,
/// never cross-verified against a real registry record) -- this type is the
/// proposal-time/acceptance-time replay/staleness-detection mechanism ADR-0023
/// discloses as not-yet-wired; the canonical DMap/2 binding field is
/// untouched by this pass. Proves only that, at binding time, `artifact_id`
/// resolved to one specific registry-computed `execution_generation`/
/// `execution_projection_digest` pair -- it does NOT prove exact
/// configuration-content pinning or secret-version identity, since the
/// registry itself does not yet provide those as verifiable authority (the
/// same honestly-disclosed limitation Stage 111/foundation-closure already
/// carry for their own equivalent checks).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DIoTArtifactAuthorityBindingV2 {
    pub artifact_id: String,
    pub execution_generation: u64,
    pub execution_projection_digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DDeployAcceptanceV2 {
    pub schema: String,
    pub acceptance_id: String,
    pub proposal_id: String,
    pub proposal_digest: String,
    pub project_id: String,
    pub application_id: String,
    pub accepted_at_utc: String,
    pub accepted_by: String,
    pub materialized_dmap_ref: ContractRefV2,
    #[serde(default)]
    pub projection_ref: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProposalV2State {
    Pending,
    Accepted,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DDeployProposalV2Record {
    pub proposal: DDeployProposalV2,
    pub proposal_digest: String,
    pub dgraph_snapshot: CanonicalDGraphV1,
    pub dcontinuum_snapshot: super::canonical_dcontinuum::CanonicalDContinuumV1,
    pub dforward_snapshot: CanonicalDForwardV1,
    pub state: ProposalV2State,
    #[serde(default)]
    pub accepted_acceptance_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveDMapV2 {
    pub map: CanonicalDMapV2,
    pub digest: String,
    pub proposal_id: String,
    pub acceptance_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct DDeployStateV2 {
    pub proposals: BTreeMap<String, DDeployProposalV2Record>,
    pub acceptances: BTreeMap<String, DDeployAcceptanceV2>,
    pub active: BTreeMap<String, ActiveDMapV2>,
}
/// Phase 113A closure pass 2: the ONE shared v1/v2 active-authority
/// interpretation every production consumer (D-Code authorization, D-Call
/// route derivation, grant issuance, grant redemption, destination
/// reauthorization) must use instead of independently reading
/// `DDeployStateV1.active_by_application`/`phase113_v2.active` -- see
/// `ddeploy::active_placement_authority`, the sole real production
/// constructor. `proposal_id`/`acceptance_id`/`dgraph_*`/`dgraph_snapshot`
/// are always `Some` from that constructor; they are only `None` for
/// `from_dmap2`'s bare-`CanonicalDMapV2` construction (used solely by the
/// realization-projection helper, which never reads them).
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementAuthorityView {
    pub project_id: String,
    pub application_id: String,
    pub dmap_id: String,
    pub revision: u64,
    pub digest: String,
    pub placements: Vec<DServPlacementV2>,
    pub proposal_id: Option<String>,
    pub acceptance_id: Option<String>,
    pub dgraph_id: Option<String>,
    pub dgraph_revision: Option<u64>,
    pub dgraph_digest: Option<String>,
    pub dgraph_snapshot: Option<CanonicalDGraphV1>,
    /// `Some` only when the active authority is DMap/2.
    pub v2: Option<PlacementAuthorityV2Extras>,
}
/// Additional fields only meaningful when the active authority is DMap/2 --
/// see `PlacementAuthorityView` doc comment.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementAuthorityV2Extras {
    pub dforward_id: String,
    pub dforward_revision: u64,
    pub dforward_digest: String,
    pub dcall_realizations: Vec<DCallRealizationV2>,
    pub diot_placements: Vec<DIoTPlacementV2>,
    /// Phase 113A closure pass 5: the EXACT accepted `CanonicalDForwardV1`
    /// this active authority was materialized against (from the immutable
    /// `DDeployProposalV2Record.dforward_snapshot`), so a governed
    /// node-projection caller never needs to accept a caller-supplied
    /// DForward as authority. `None` only for `PlacementAuthorityView::
    /// from_dmap2`'s bare-`CanonicalDMapV2` construction path (which has no
    /// proposal context to source a snapshot from and is not used by the
    /// governed projection entry point).
    pub dforward_snapshot: Option<CanonicalDForwardV1>,
    /// Phase 113A closure pass 5: `CanonicalDMapV2.external_port_bindings`,
    /// exposed here for the same governed-projection reason as the other
    /// fields on this type -- omitted from the original Pass 2 extension
    /// because no consumer needed it until node projection did.
    pub external_port_bindings: Vec<ExternalPortBindingV2>,
}
impl DDeployProposalV2 {
    pub fn schema_valid(&self) -> bool {
        self.schema == DDEPLOY_PROPOSAL_V2_SCHEMA
            && !self.proposal_id.trim().is_empty()
            && !self.project_id.trim().is_empty()
            && !self.application_id.trim().is_empty()
            // Phase 113A closure pass 4.1: server-generated created_at_utc
            // must be a genuine RFC3339 timestamp, mirroring v1's identical
            // `validate_proposal_shape` check.
            && chrono::DateTime::parse_from_rfc3339(&self.created_at_utc).is_ok()
    }
}
impl DDeployAcceptanceV2 {
    pub fn schema_valid(&self) -> bool {
        self.schema == DDEPLOY_ACCEPTANCE_V2_SCHEMA
            && !self.acceptance_id.trim().is_empty()
            && !self.proposal_id.trim().is_empty()
            && !self.accepted_by.trim().is_empty()
            && !self.accepted_at_utc.trim().is_empty()
    }
}
impl DDeployStateV2 {
    pub fn proposal_digest(p: &DDeployProposalV2) -> Result<String, String> {
        let v: Value = serde_json::to_value(p).map_err(|e| e.to_string())?;
        jcs::canonical_digest_sha256_hex(&v).map_err(|e| e.to_string())
    }
    pub fn create_proposal(
        &mut self,
        proposal: DDeployProposalV2,
        dg: CanonicalDGraphV1,
        dc: super::canonical_dcontinuum::CanonicalDContinuumV1,
        df: CanonicalDForwardV1,
    ) -> Result<String, String> {
        if !proposal.schema_valid() {
            return Err("invalid ddeploy v2 proposal".into());
        }
        if self.proposals.contains_key(&proposal.proposal_id) {
            return Err("proposal is immutable and already exists".into());
        }
        // Phase 113A closure pass 3: internal consistency only -- every DIoT
        // requirement must have SOME binding entry keyed by its artifact_id.
        // This does NOT re-verify the binding against a live registry (this
        // type has no registry access); that real verification is
        // `ddeploy::resolve_diot_artifact_authority`'s job, which the caller
        // must have already run to populate `proposal.diot_artifact_authority`
        // before constructing this value.
        for diot in &df.diots {
            if !proposal
                .diot_artifact_authority
                .contains_key(&diot.artifact_requirement.artifact_id)
            {
                return Err(format!(
                    "diot '{}' requires artifact '{}', which has no resolved diot_artifact_authority binding",
                    diot.diot_id, diot.artifact_requirement.artifact_id
                ));
            }
        }
        let digest = Self::proposal_digest(&proposal)?;
        let id = proposal.proposal_id.clone();
        self.proposals.insert(
            id.clone(),
            DDeployProposalV2Record {
                proposal,
                proposal_digest: digest.clone(),
                dgraph_snapshot: dg,
                dcontinuum_snapshot: dc,
                dforward_snapshot: df,
                state: ProposalV2State::Pending,
                accepted_acceptance_id: None,
            },
        );
        Ok(digest)
    }
    pub fn accept(
        &mut self,
        proposal_id: &str,
        acceptance_id: String,
        accepted_by: String,
        accepted_at_utc: String,
    ) -> Result<DDeployAcceptanceV2, String> {
        self.accept_with_previous(
            proposal_id,
            acceptance_id,
            accepted_by,
            accepted_at_utc,
            None,
        )
    }

    /// Accept using a previous authority from the shared DDeploy domain.
    /// This is what makes a v1 -> v2 transition retain the logical identity
    /// and monotonic revision without creating a second authority.
    pub fn accept_with_previous(
        &mut self,
        proposal_id: &str,
        acceptance_id: String,
        accepted_by: String,
        accepted_at_utc: String,
        previous: Option<(&str, u64)>,
    ) -> Result<DDeployAcceptanceV2, String> {
        let rec = self
            .proposals
            .get(proposal_id)
            .ok_or("proposal not found")?
            .clone();
        if Self::proposal_digest(&rec.proposal)? != rec.proposal_digest {
            return Err("proposal digest mismatch".into());
        }
        if let Some(existing) = self.acceptances.get(&acceptance_id) {
            if existing.proposal_id == proposal_id
                && existing.proposal_digest == rec.proposal_digest
            {
                return Ok(existing.clone());
            }
            return Err("acceptance_id already belongs to another proposal".into());
        }
        if let Some(a) = self
            .acceptances
            .values()
            .find(|a| a.proposal_id == proposal_id)
        {
            return Ok(a.clone());
        }
        let key =
            super::ddeploy::application_key(&rec.proposal.project_id, &rec.proposal.application_id);
        let previous_local = self.active.get(&key);
        let previous_revision = previous
            .map(|(_, revision)| revision)
            .or_else(|| previous_local.map(|x| x.map.revision));
        if rec.proposal.expected_active_dmap_revision != previous_revision {
            return Err("stale active DMap revision".into());
        }
        let mut map = CanonicalDMapV2 {
            schema: super::canonical_dmap_v2::CANONICAL_DMAP_V2_SCHEMA.into(),
            project_id: rec.proposal.project_id.clone(),
            application_id: rec.proposal.application_id.clone(),
            dmap_id: previous
                .map(|(id, _)| id.to_string())
                .or_else(|| previous_local.map(|x| x.map.dmap_id.clone()))
                .unwrap_or_else(|| {
                    super::ddeploy::canonical_dmap_id(
                        &rec.proposal.project_id,
                        &rec.proposal.application_id,
                    )
                }),
            revision: previous_revision.map(|revision| revision + 1).unwrap_or(1),
            dgraph_ref: rec.proposal.dgraph_ref.clone(),
            dcontinuum_ref: rec.proposal.dcontinuum_ref.clone(),
            dforward_ref: rec.proposal.dforward_ref.clone(),
            placements: rec.proposal.placements.clone(),
            diot_placements: rec.proposal.diot_placements.clone(),
            dcall_realizations: rec.proposal.dcall_realizations.clone(),
            external_port_bindings: rec.proposal.external_port_bindings.clone(),
        };
        if !map
            .validate(
                &rec.dgraph_snapshot,
                &rec.dcontinuum_snapshot,
                &rec.dforward_snapshot,
            )
            .is_empty()
        {
            return Err("materialized DMap/2 failed validation".into());
        }
        let digest = map.digest()?;
        let a = DDeployAcceptanceV2 {
            schema: DDEPLOY_ACCEPTANCE_V2_SCHEMA.into(),
            acceptance_id: acceptance_id.clone(),
            proposal_id: proposal_id.into(),
            proposal_digest: rec.proposal_digest,
            project_id: map.project_id.clone(),
            application_id: map.application_id.clone(),
            accepted_at_utc,
            accepted_by,
            materialized_dmap_ref: ContractRefV2 {
                id: map.dmap_id.clone(),
                revision: map.revision,
                digest: digest.clone(),
            },
            projection_ref: None,
        };
        if !a.schema_valid() {
            return Err("invalid acceptance".into());
        }
        self.active.insert(
            key,
            ActiveDMapV2 {
                map,
                digest,
                proposal_id: proposal_id.into(),
                acceptance_id: acceptance_id.clone(),
            },
        );
        self.acceptances.insert(acceptance_id, a.clone());
        if let Some(r) = self.proposals.get_mut(proposal_id) {
            r.state = ProposalV2State::Accepted;
            r.accepted_acceptance_id = Some(a.acceptance_id.clone())
        }
        Ok(a)
    }

    pub fn validate_integrity(&self) -> Result<(), String> {
        for (id, record) in &self.proposals {
            if id != &record.proposal.proposal_id {
                return Err("v2 proposal key mismatch".into());
            }
            // Phase 113A closure pass 4.1: re-validate created_at_utc's
            // RFC3339 shape on every reload, not just at creation time --
            // catches a persisted record whose timestamp was corrupted
            // independently of (or in addition to) a digest mismatch.
            if !record.proposal.schema_valid() {
                return Err(format!(
                    "v2 proposal '{id}' failed schema_valid() on reload"
                ));
            }
            if Self::proposal_digest(&record.proposal)? != record.proposal_digest {
                return Err(format!("v2 proposal '{id}' digest mismatch"));
            }
            if record.state == ProposalV2State::Pending && record.accepted_acceptance_id.is_some() {
                return Err("pending v2 proposal has acceptance".into());
            }
            if record.state == ProposalV2State::Accepted {
                let aid = record
                    .accepted_acceptance_id
                    .as_ref()
                    .ok_or("accepted v2 proposal missing acceptance")?;
                let a = self.acceptances.get(aid).ok_or("v2 acceptance missing")?;
                if a.proposal_id != *id || a.proposal_digest != record.proposal_digest {
                    return Err("v2 acceptance ownership mismatch".into());
                }
            }
        }
        for (id, acceptance) in &self.acceptances {
            if id != &acceptance.acceptance_id {
                return Err("v2 acceptance key mismatch".into());
            }
            // Proposals are never deleted by this module -- a dangling
            // `proposal_id` is corruption, not a tolerated gap (mirrors the
            // v1 precedent in `ddeploy.rs`'s category-B check, which its own
            // comment notes v2 previously failed to carry over).
            let referenced_proposal = self.proposals.get(&acceptance.proposal_id).ok_or_else(|| {
                format!(
                    "v2 acceptance '{id}' references proposal '{}', which does not exist in state.phase113_v2.proposals",
                    acceptance.proposal_id
                )
            })?;
            if referenced_proposal.proposal.proposal_id != acceptance.proposal_id {
                return Err(format!(
                    "v2 acceptance '{id}' proposal_id '{}' does not match referenced proposal's own proposal_id '{}'",
                    acceptance.proposal_id, referenced_proposal.proposal.proposal_id
                ));
            }
            if referenced_proposal.proposal_digest != acceptance.proposal_digest {
                return Err(format!(
                    "v2 acceptance '{id}' proposal_digest '{}' does not match referenced proposal '{}''s recorded proposal_digest '{}'",
                    acceptance.proposal_digest, acceptance.proposal_id, referenced_proposal.proposal_digest
                ));
            }
            if referenced_proposal.proposal.project_id != acceptance.project_id
                || referenced_proposal.proposal.application_id != acceptance.application_id
            {
                return Err(format!(
                    "v2 acceptance '{id}' project/application ('{}','{}') does not match referenced proposal '{}''s own ('{}','{}')",
                    acceptance.project_id, acceptance.application_id, acceptance.proposal_id,
                    referenced_proposal.proposal.project_id, referenced_proposal.proposal.application_id
                ));
            }
            if referenced_proposal.accepted_acceptance_id.as_deref() != Some(id.as_str()) {
                return Err(format!(
                    "v2 acceptance '{id}' references proposal '{}', but that proposal's accepted_acceptance_id is '{:?}', not '{id}' -- an acceptance must point to a proposal that points back to it",
                    acceptance.proposal_id, referenced_proposal.accepted_acceptance_id
                ));
            }
        }
        for (key, active) in &self.active {
            let expected =
                super::ddeploy::application_key(&active.map.project_id, &active.map.application_id);
            if key != &expected {
                return Err("v2 active key mismatch".into());
            }
            if active.map.digest()? != active.digest {
                return Err("v2 active digest mismatch".into());
            }
            let acceptance = self
                .acceptances
                .get(&active.acceptance_id)
                .ok_or("v2 active acceptance missing")?;
            if acceptance.acceptance_id != active.acceptance_id {
                return Err(format!(
                    "v2 active '{key}' acceptance_id '{}' does not match referenced acceptance's own acceptance_id '{}'",
                    active.acceptance_id, acceptance.acceptance_id
                ));
            }
            if acceptance.proposal_id != active.proposal_id
                || acceptance.project_id != active.map.project_id
                || acceptance.application_id != active.map.application_id
                || acceptance.materialized_dmap_ref.id != active.map.dmap_id
                || acceptance.materialized_dmap_ref.digest != active.digest
                || acceptance.materialized_dmap_ref.revision != active.map.revision
            {
                return Err("v2 active relational mismatch".into());
            }
            // Do not stop at active<->acceptance consistency: an active
            // authority whose acceptance's proposal has vanished (or drifted)
            // would silently lose the immutable governance evidence --
            // including `diot_artifact_authority` -- that justified it.
            let referenced_proposal = self.proposals.get(&active.proposal_id).ok_or_else(|| {
                format!(
                    "v2 active '{key}' references proposal '{}', which does not exist in state.phase113_v2.proposals",
                    active.proposal_id
                )
            })?;
            if referenced_proposal.proposal.proposal_id != active.proposal_id {
                return Err(format!(
                    "v2 active '{key}' proposal_id '{}' does not match referenced proposal's own proposal_id '{}'",
                    active.proposal_id, referenced_proposal.proposal.proposal_id
                ));
            }
            if referenced_proposal.proposal_digest != acceptance.proposal_digest {
                return Err(format!(
                    "v2 active '{key}' acceptance proposal_digest '{}' does not match referenced proposal '{}''s recorded proposal_digest '{}'",
                    acceptance.proposal_digest, active.proposal_id, referenced_proposal.proposal_digest
                ));
            }
        }
        Ok(())
    }
}
impl PlacementAuthorityView {
    pub fn from_dmap2(m: &CanonicalDMapV2) -> Result<Self, String> {
        Ok(Self {
            project_id: m.project_id.clone(),
            application_id: m.application_id.clone(),
            dmap_id: m.dmap_id.clone(),
            revision: m.revision,
            digest: m.digest()?,
            placements: m.placements.clone(),
            proposal_id: None,
            acceptance_id: None,
            dgraph_id: None,
            dgraph_revision: None,
            dgraph_digest: None,
            dgraph_snapshot: None,
            v2: Some(PlacementAuthorityV2Extras {
                dforward_id: m.dforward_ref.id.clone(),
                dforward_revision: m.dforward_ref.revision,
                dforward_digest: m.dforward_ref.digest.clone(),
                dcall_realizations: m.dcall_realizations.clone(),
                diot_placements: m.diot_placements.clone(),
                // `from_dmap2` has no proposal context to source a real
                // DForward snapshot from (see the field's own doc comment);
                // its only caller passes `forward` as a separate parameter.
                dforward_snapshot: None,
                external_port_bindings: m.external_port_bindings.clone(),
            }),
        })
    }
}
