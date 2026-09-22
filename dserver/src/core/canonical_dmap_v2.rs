//! `datum.dmap/2`: canonical DServ placement plus DIoT realization authority.
use crate::core::{
    canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1},
    canonical_dforward::CanonicalDForwardV1,
    canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1, CanonicalValidationFinding},
    canonical_dmap::{CanonicalContractReferenceV1, CanonicalDMapV1, CanonicalDServPlacementV1},
    jcs,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const CANONICAL_DMAP_V2_SCHEMA: &str = "datum.dmap/2";
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContractRefV2 {
    pub id: String,
    pub revision: u64,
    pub digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DServPlacementV2 {
    pub service_id: String,
    pub node_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DIoTArtifactBindingV2 {
    pub artifact_id: String,
    pub descriptor_digest: String,
    pub content_digest: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DIoTPlacementV2 {
    pub placement_id: String,
    pub diot_id: String,
    pub node_id: String,
    pub artifact_binding: DIoTArtifactBindingV2,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DCallRealizationV2 {
    pub call_id: String,
    pub realization_kind: String,
    pub diot_placement_id: String,
    pub interface: String,
    pub channel: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExternalPortBindingV2 {
    pub service_id: String,
    pub port: String,
    pub endpoint_id: String,
    pub diot_placement_id: Option<String>,
    pub interface: String,
    pub channel: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDMapV2 {
    pub schema: String,
    pub project_id: String,
    pub application_id: String,
    pub dmap_id: String,
    pub revision: u64,
    pub dgraph_ref: ContractRefV2,
    pub dcontinuum_ref: ContractRefV2,
    pub dforward_ref: ContractRefV2,
    #[serde(default)]
    pub placements: Vec<DServPlacementV2>,
    #[serde(default)]
    pub diot_placements: Vec<DIoTPlacementV2>,
    #[serde(default)]
    pub dcall_realizations: Vec<DCallRealizationV2>,
    #[serde(default)]
    pub external_port_bindings: Vec<ExternalPortBindingV2>,
}
impl CanonicalDMapV2 {
    pub fn validate_against(
        &self,
        dg: &CanonicalDGraphV1,
        dc: &CanonicalDContinuumV1,
        df: &CanonicalDForwardV1,
    ) -> Vec<CanonicalValidationFinding> {
        self.validate(dg, dc, df)
    }
    pub fn validate(
        &self,
        dg: &CanonicalDGraphV1,
        dc: &CanonicalDContinuumV1,
        df: &CanonicalDForwardV1,
    ) -> Vec<CanonicalValidationFinding> {
        let mut e = Vec::new();
        // Reuse the complete DMap/1 DServ placement gate so ABI, individual
        // capacity, aggregate capacity, known-node/service, and all-DServ
        // placement invariants cannot drift between wire versions.
        let shared = CanonicalDMapV1 {
            schema: super::canonical_dmap::CANONICAL_DMAP_SCHEMA.into(),
            project_id: self.project_id.clone(),
            application_id: self.application_id.clone(),
            dmap_id: self.dmap_id.clone(),
            revision: self.revision,
            dgraph_ref: CanonicalContractReferenceV1 {
                id: self.dgraph_ref.id.clone(),
                revision: self.dgraph_ref.revision,
                digest: self.dgraph_ref.digest.clone(),
            },
            dcontinuum_ref: CanonicalContractReferenceV1 {
                id: self.dcontinuum_ref.id.clone(),
                revision: self.dcontinuum_ref.revision,
                digest: self.dcontinuum_ref.digest.clone(),
            },
            placements: self
                .placements
                .iter()
                .map(|p| CanonicalDServPlacementV1 {
                    service_id: p.service_id.clone(),
                    node_id: p.node_id.clone(),
                })
                .collect(),
        };
        e.extend(shared.validate_against(dg, dc));
        e.extend(dg.validate());
        e.extend(dc.validate());
        e.extend(df.validate());
        if self.project_id != dc.project_id {
            e.push(finding(
                "project_mismatch",
                "project_id must match DContinuum",
            ));
        }
        if self.application_id != dg.application_id {
            e.push(finding(
                "application_mismatch",
                "application_id must match DGraph",
            ));
        }
        if self.schema != CANONICAL_DMAP_V2_SCHEMA {
            e.push(finding("schema_invalid", "schema must be datum.dmap/2"));
        }
        if self.project_id.trim().is_empty()
            || self.application_id.trim().is_empty()
            || self.dmap_id.trim().is_empty()
        {
            e.push(finding(
                "missing_field",
                "project_id/application_id/dmap_id must be non-empty",
            ));
        }
        if self.revision < 1 {
            e.push(finding("revision_invalid", "revision must be >= 1"));
        }
        for (n, r) in [
            ("dgraph", &self.dgraph_ref),
            ("dcontinuum", &self.dcontinuum_ref),
            ("dforward", &self.dforward_ref),
        ] {
            if r.id.trim().is_empty() || r.revision < 1 {
                e.push(finding(
                    "reference_invalid",
                    format!("{n} reference is incomplete"),
                ));
            }
            if !digest_ok(&r.digest) {
                e.push(finding(
                    "invalid_digest_format",
                    format!("{n} reference digest is malformed"),
                ));
            }
        }
        if self.dgraph_ref.id != dg.dgraph_id
            || self.dgraph_ref.revision != dg.revision
            || self.dcontinuum_ref.id != dc.dcontinuum_id
            || self.dcontinuum_ref.revision != dc.revision
            || self.dforward_ref.id != df.dforward_id
            || self.dforward_ref.revision != df.revision
        {
            e.push(finding(
                "reference_mismatch",
                "DMap/2 contract references do not match supplied contracts",
            ));
        }
        if let Ok(actual) = recompute_dgraph_digest(dg) {
            if actual != self.dgraph_ref.digest {
                e.push(finding(
                    "dgraph_digest_mismatch",
                    "DGraph content digest does not match",
                ));
            }
        }
        if let Ok(actual) = recompute_dcontinuum_digest(dc) {
            if actual != self.dcontinuum_ref.digest {
                e.push(finding(
                    "dcontinuum_digest_mismatch",
                    "DContinuum content digest does not match",
                ));
            }
        }
        if let Ok(actual) = df.digest() {
            if actual != self.dforward_ref.digest {
                e.push(finding(
                    "dforward_digest_mismatch",
                    "DForward content digest does not match",
                ));
            }
        }
        let mut services = std::collections::HashSet::new();
        for p in &self.placements {
            if !services.insert(&p.service_id) {
                e.push(finding("duplicate_placement", "duplicate DServ placement"));
            }
            if dg.services.iter().all(|s| s.service_id != p.service_id) {
                e.push(finding(
                    "unknown_service",
                    "DServ placement names unknown service",
                ));
            }
            if dc.nodes.iter().all(|n| n.node_id != p.node_id) {
                e.push(finding(
                    "unknown_node",
                    "DServ placement names unknown node",
                ));
            }
        }
        for s in &dg.services {
            if self.placements.iter().all(|p| p.service_id != s.service_id) {
                e.push(finding("missing_placement", "DGraph service is not placed"));
            }
        }
        let mut diot_ids = std::collections::HashSet::new();
        for p in &self.diot_placements {
            if !diot_ids.insert(&p.placement_id) {
                e.push(finding(
                    "duplicate_diot_placement",
                    "duplicate DIoT placement_id",
                ));
            }
            if df.diots.iter().all(|d| d.diot_id != p.diot_id) {
                e.push(finding("unknown_diot", "DIoT placement names unknown DIoT"));
            }
            if let Some(d) = df.diots.iter().find(|d| d.diot_id == p.diot_id) {
                if d.artifact_requirement.artifact_id != p.artifact_binding.artifact_id {
                    e.push(finding(
                        "artifact_requirement_mismatch",
                        "DIoT artifact binding does not match DForward requirement",
                    ));
                }
            }
            if dc.nodes.iter().all(|n| n.node_id != p.node_id) {
                e.push(finding("unknown_node", "DIoT placement names unknown node"));
            }
            if p.artifact_binding.artifact_id.trim().is_empty()
                || !digest_ok(&p.artifact_binding.descriptor_digest)
            {
                e.push(finding(
                    "artifact_binding_invalid",
                    "DIoT artifact binding is incomplete",
                ));
            }
            if p.artifact_binding
                .content_digest
                .as_ref()
                .is_some_and(|d| !digest_ok(d))
            {
                e.push(finding(
                    "artifact_binding_invalid",
                    "DIoT content digest is malformed",
                ));
            }
        }
        let mut calls = std::collections::HashSet::new();
        for r in &self.dcall_realizations {
            if !calls.insert(&r.call_id) {
                e.push(finding(
                    "duplicate_dcall_realization",
                    "duplicate DCall realization",
                ));
            }
            if dg.calls.iter().all(|c| c.call_id != r.call_id) {
                e.push(finding(
                    "unknown_dcall",
                    "DCall realization names unknown call",
                ));
            }
            if r.realization_kind != "via_diot_pubsub"
                || r.realization_kind.trim().is_empty()
                || r.interface.trim().is_empty()
                || r.channel.trim().is_empty()
            {
                e.push(finding(
                    "realization_invalid",
                    "DCall realization needs kind/interface/channel",
                ));
            }
            if r.channel.contains('+') || r.channel.contains('#') {
                e.push(finding(
                    "channel_malformed",
                    "wildcard channel is forbidden",
                ));
            }
            if !diot_ids.contains(&r.diot_placement_id) {
                e.push(finding(
                    "unknown_diot_placement",
                    "DCall realization names unknown DIoT placement",
                ));
            }
            if let Some(d) = df.diots.iter().find(|d| {
                self.diot_placements
                    .iter()
                    .any(|p| p.placement_id == r.diot_placement_id && p.diot_id == d.diot_id)
            }) {
                let has = d.provides.iter().any(|x| x.interface == r.interface)
                    || d.requires.iter().any(|x| x.interface == r.interface);
                if !has {
                    e.push(finding(
                        "interface_unknown",
                        "realization interface is absent from DIoT semantics",
                    ));
                }
            }
        }
        for c in &dg.calls {
            if self
                .dcall_realizations
                .iter()
                .filter(|r| r.call_id == c.call_id)
                .count()
                != 1
            {
                e.push(finding(
                    "missing_dcall_realization",
                    "each DGraph call requires exactly one realization",
                ));
            }
        }
        let mut external_keys = std::collections::HashSet::new();
        for b in &self.external_port_bindings {
            if !external_keys.insert((&b.service_id, &b.port, &b.endpoint_id)) {
                e.push(finding(
                    "duplicate_external_binding",
                    "duplicate external binding",
                ));
            }
            if b.service_id.trim().is_empty()
                || b.port.trim().is_empty()
                || b.endpoint_id.trim().is_empty()
                || b.interface.trim().is_empty()
                || b.channel.trim().is_empty()
            {
                e.push(finding(
                    "external_binding_invalid",
                    "external binding is incomplete",
                ));
            }
            if b.channel.contains('+') || b.channel.contains('#') {
                e.push(finding(
                    "channel_malformed",
                    "wildcard channel is forbidden",
                ));
            }
            if dg.services.iter().all(|s| s.service_id != b.service_id) {
                e.push(finding(
                    "unknown_service",
                    "external binding names unknown service",
                ));
            }
            if df
                .external_endpoints
                .iter()
                .all(|x| x.endpoint_id != b.endpoint_id)
            {
                e.push(finding(
                    "unknown_external_endpoint",
                    "external binding names unknown endpoint",
                ));
            }
            if let Some(endpoint) = df
                .external_endpoints
                .iter()
                .find(|x| x.endpoint_id == b.endpoint_id)
            {
                if endpoint.interface != b.interface {
                    e.push(finding(
                        "external_interface_mismatch",
                        "external binding interface differs from DForward endpoint",
                    ));
                }
                if let Some(pid) = &b.diot_placement_id {
                    if let Some(p) = self.diot_placements.iter().find(|p| &p.placement_id == pid) {
                        if let Some(d) = df.diots.iter().find(|d| d.diot_id == p.diot_id) {
                            if !d.provides.iter().any(|x| x.interface == b.interface)
                                && !d.requires.iter().any(|x| x.interface == b.interface)
                            {
                                e.push(finding(
                                    "interface_unknown",
                                    "external binding interface absent from adjacent DIoT",
                                ));
                            }
                        }
                    }
                }
            }
            if b.diot_placement_id
                .as_ref()
                .is_some_and(|id| !diot_ids.contains(id))
            {
                e.push(finding(
                    "unknown_diot_placement",
                    "external binding names unknown DIoT placement",
                ));
            }
        }
        e
    }
    pub fn digest(&self) -> Result<String, String> {
        let mut x = self.clone();
        x.placements.sort_by(|a, b| a.service_id.cmp(&b.service_id));
        x.diot_placements
            .sort_by(|a, b| a.placement_id.cmp(&b.placement_id));
        x.dcall_realizations
            .sort_by(|a, b| a.call_id.cmp(&b.call_id));
        x.external_port_bindings.sort_by(|a, b| {
            (
                a.service_id.as_str(),
                a.port.as_str(),
                a.endpoint_id.as_str(),
                a.diot_placement_id.as_deref().unwrap_or(""),
                a.interface.as_str(),
                a.channel.as_str(),
            )
                .cmp(&(
                    b.service_id.as_str(),
                    b.port.as_str(),
                    b.endpoint_id.as_str(),
                    b.diot_placement_id.as_deref().unwrap_or(""),
                    b.interface.as_str(),
                    b.channel.as_str(),
                ))
        });
        let v: Value = serde_json::to_value(x).map_err(|e| e.to_string())?;
        jcs::canonical_digest_sha256_hex(&v).map_err(|e| e.to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// `CanonicalDMapV2` (and `DDeployProposalV2`/`DDeployAcceptanceV2`)
    /// have `#[serde(deny_unknown_fields)]`, unlike `CanonicalDGraphV1`/
    /// `CanonicalDContinuumV1`, which silently drop unrecognised keys (see
    /// `scripts/check_canonical_datum_contracts_v1.py`'s own
    /// `digest_of_dgraph` doc comment on this exact asymmetry). Fixture
    /// bookkeeping metadata (`x_note`, `expected_digest_sha256`) must
    /// therefore be stripped before deserializing into a `deny_unknown_fields`
    /// struct -- this helper mirrors that same, already-established
    /// convention for the strict Phase 113A wire types.
    fn strip_fixture_bookkeeping(raw: &str) -> String {
        let mut v: Value = serde_json::from_str(raw).unwrap();
        if let Some(obj) = v.as_object_mut() {
            obj.remove("x_note");
            obj.remove("expected_digest_sha256");
        }
        serde_json::to_string(&v).unwrap()
    }
    #[test]
    fn real_positive_fixture_deserializes_and_validates_against_the_real_bundle() {
        // Phase 113A closure pass 5, section 11: load the real positive
        // datum.dmap/2 fixture, deserialize it into the actual Rust wire
        // type, and validate it against the REAL dgraph/dcontinuum/dforward
        // fixtures it claims to be consistent with -- proving the bundle is
        // genuinely mutually consistent end-to-end, not merely
        // schema-shaped.
        let dgraph: CanonicalDGraphV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dgraph.smart-sentinel-lab.json"
        ))
        .unwrap();
        let dcontinuum: CanonicalDContinuumV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dcontinuum.smart-sentinel-lab.json"
        ))
        .unwrap();
        let dforward: CanonicalDForwardV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        ))
        .unwrap();
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let cleaned = strip_fixture_bookkeeping(raw);
        let map: CanonicalDMapV2 = serde_json::from_str(&cleaned)
            .expect("real positive datum.dmap/2 fixture must deserialize into CanonicalDMapV2");
        let findings = map.validate(&dgraph, &dcontinuum, &dforward);
        assert!(findings.is_empty(), "real positive DMap/2 bundle must validate cleanly against its real DGraph/DContinuum/DForward: {findings:?}");
        assert_eq!(
            map.digest().unwrap(),
            "sha256:b38a2aa111f358cb92fda232d6a5f707f798b6e4dca278626a7e0dd5a7e39851",
            "fixture's own embedded expected_digest_sha256 must match the real recomputed digest"
        );
    }
    #[test]
    fn real_positive_fixture_ordering_permutation_produces_identical_digest() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let cleaned = strip_fixture_bookkeeping(raw);
        let mut a: CanonicalDMapV2 = serde_json::from_str(&cleaned).unwrap();
        let mut b = a.clone();
        b.placements.reverse();
        b.diot_placements.reverse();
        b.dcall_realizations.reverse();
        b.external_port_bindings.reverse();
        assert_eq!(a.digest().unwrap(), b.digest().unwrap());
        a.dmap_id = "different".into();
        assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    }
    #[test]
    fn wrong_schema_is_rejected() {
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
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let cleaned = strip_fixture_bookkeeping(raw);
        let mut map: CanonicalDMapV2 = serde_json::from_str(&cleaned).unwrap();
        map.schema = "datum.dmap/1".into();
        assert!(map
            .validate(&dgraph, &dcontinuum, &dforward)
            .iter()
            .any(|f| f.code == "schema_invalid"));
    }
    #[test]
    fn unknown_field_is_rejected_by_deny_unknown_fields() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let mut v: Value = serde_json::from_str(raw).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("x_note");
        obj.remove("expected_digest_sha256");
        obj.insert("rogue_field".into(), Value::Bool(true));
        let text = serde_json::to_string(&v).unwrap();
        let result: Result<CanonicalDMapV2, _> = serde_json::from_str(&text);
        assert!(
            result.is_err(),
            "an unrecognised field must be rejected by deny_unknown_fields"
        );
    }
    #[test]
    fn digest_is_order_independent() {
        let r = ContractRefV2 {
            id: "x".into(),
            revision: 1,
            digest: "sha256:".to_string() + &"0".repeat(64),
        };
        let mut a = CanonicalDMapV2 {
            schema: CANONICAL_DMAP_V2_SCHEMA.into(),
            project_id: "p".into(),
            application_id: "a".into(),
            dmap_id: "m".into(),
            revision: 1,
            dgraph_ref: r.clone(),
            dcontinuum_ref: r.clone(),
            dforward_ref: r,
            placements: vec![
                DServPlacementV2 {
                    service_id: "b".into(),
                    node_id: "n".into(),
                },
                DServPlacementV2 {
                    service_id: "a".into(),
                    node_id: "n".into(),
                },
            ],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
        };
        let mut b = a.clone();
        b.placements.reverse();
        assert_eq!(a.digest().unwrap(), b.digest().unwrap());
        a.application_id = "other".into();
        assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    }
    #[test]
    fn duplicate_placement_is_rejected() {
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
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let cleaned = strip_fixture_bookkeeping(raw);
        let mut map: CanonicalDMapV2 = serde_json::from_str(&cleaned).unwrap();
        let dup = map.placements[0].clone();
        map.placements.push(dup);
        assert!(map
            .validate(&dgraph, &dcontinuum, &dforward)
            .iter()
            .any(|f| f.code == "duplicate_placement"));
    }
    #[test]
    fn wildcard_channel_is_rejected() {
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
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let cleaned = strip_fixture_bookkeeping(raw);
        let mut map: CanonicalDMapV2 = serde_json::from_str(&cleaned).unwrap();
        map.dcall_realizations[0].channel = "iot/#".into();
        assert!(map
            .validate(&dgraph, &dcontinuum, &dforward)
            .iter()
            .any(|f| f.code == "channel_malformed"));
    }
    #[test]
    fn every_negative_dmap2_fixture_actually_produces_its_claimed_finding() {
        // Phase 113A closure pass 5, section 13: a negative fixture's
        // claimed `x_expect_reject.code` must be genuinely reproducible by
        // the REAL Rust validator (or, for the `deny_unknown_fields`
        // sentinel, by real deserialization failure), not merely asserted
        // in fixture prose. Mirrors
        // `canonical_dforward::every_negative_dforward_fixture_actually_produces_its_claimed_finding`.
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
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/datum-contracts/dmap-v2/negative");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("negative fixture dir must exist") {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.starts_with("dmap.") || !name.ends_with(".json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let mut v: Value = serde_json::from_str(&raw).unwrap();
            let obj = v.as_object_mut().unwrap();
            obj.remove("x_note");
            obj.remove("expected_digest_sha256");
            let expected_code = obj
                .remove("x_expect_reject")
                .and_then(|x| x.get("code").cloned())
                .and_then(|c| c.as_str().map(str::to_string))
                .unwrap_or_else(|| {
                    panic!("{name}: negative fixture must carry x_expect_reject.code")
                });
            let cleaned = serde_json::to_string(&v).unwrap();
            if expected_code == "deny_unknown_fields" {
                let result: Result<CanonicalDMapV2, _> = serde_json::from_str(&cleaned);
                assert!(
                    result.is_err(),
                    "{name}: fixture claims deny_unknown_fields rejection but deserialized successfully"
                );
                checked += 1;
                continue;
            }
            let map: CanonicalDMapV2 = serde_json::from_str(&cleaned).unwrap_or_else(|e| {
                panic!("{name}: negative fixture must still deserialize into the real type: {e}")
            });
            let findings = map.validate(&dgraph, &dcontinuum, &dforward);
            assert!(
                findings.iter().any(|f| f.code == expected_code),
                "{name}: expected finding code '{expected_code}' was not produced -- got {findings:?}"
            );
            checked += 1;
        }
        assert!(
            checked >= 12,
            "expected at least 12 real dmap/2 negative fixtures to be exercised, found {checked}"
        );
    }
}
