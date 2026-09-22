//! SmartSentinel mirror of `datum.dmap/2`.
pub use crate::agent::canonical_dforward::{CanonicalDForwardV1, CanonicalDIoTV1};
use crate::agent::{canonical_dgraph::CanonicalValidationFinding, jcs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const CANONICAL_DMAP_V2_SCHEMA: &str = "datum.dmap/2";
fn f(c: &'static str, m: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code: c,
        message: m.into(),
    }
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
fn dg(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|x| {
        x.len() == 64
            && x.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}
impl CanonicalDMapV2 {
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
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut e = Vec::new();
        if self.schema != CANONICAL_DMAP_V2_SCHEMA {
            e.push(f("schema_invalid", "schema must be datum.dmap/2"));
        }
        if self.project_id.trim().is_empty()
            || self.application_id.trim().is_empty()
            || self.dmap_id.trim().is_empty()
        {
            e.push(f("missing_field", "identity fields must be non-empty"));
        }
        if self.revision < 1 {
            e.push(f("revision_invalid", "revision must be >= 1"));
        }
        for r in [&self.dgraph_ref, &self.dcontinuum_ref, &self.dforward_ref] {
            if r.id.trim().is_empty() || r.revision < 1 || !dg(&r.digest) {
                e.push(f(
                    "reference_invalid",
                    "reference id/revision/digest is invalid",
                ));
            }
        }
        let mut s = std::collections::HashSet::new();
        for p in &self.placements {
            if p.service_id.trim().is_empty() || p.node_id.trim().is_empty() {
                e.push(f(
                    "placement_invalid",
                    "DServ placement identity is incomplete",
                ));
            }
            if !s.insert(&p.service_id) {
                e.push(f("duplicate_placement", "duplicate DServ placement"));
            }
        }
        let mut d = std::collections::HashSet::new();
        for p in &self.diot_placements {
            if p.placement_id.trim().is_empty()
                || p.diot_id.trim().is_empty()
                || p.node_id.trim().is_empty()
            {
                e.push(f(
                    "diot_placement_invalid",
                    "DIoT placement identity is incomplete",
                ));
            }
            if !d.insert(&p.placement_id) {
                e.push(f("duplicate_diot_placement", "duplicate DIoT placement"));
            }
            if p.artifact_binding.artifact_id.trim().is_empty()
                || !dg(&p.artifact_binding.descriptor_digest)
            {
                e.push(f(
                    "artifact_binding_invalid",
                    "artifact binding is incomplete",
                ));
            }
        }
        let mut c = std::collections::HashSet::new();
        for r in &self.dcall_realizations {
            if !c.insert(&r.call_id) {
                e.push(f(
                    "duplicate_dcall_realization",
                    "duplicate DCall realization",
                ));
            }
            if r.realization_kind != "via_diot_pubsub"
                || r.diot_placement_id.trim().is_empty()
                || r.interface.trim().is_empty()
                || r.channel.trim().is_empty()
                || r.channel.contains('+')
                || r.channel.contains('#')
            {
                e.push(f(
                    "realization_invalid",
                    "realization kind/interface/channel is invalid",
                ));
            }
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn strip_fixture_bookkeeping(raw: &str) -> String {
        let mut v: Value = serde_json::from_str(raw).unwrap();
        if let Some(obj) = v.as_object_mut() {
            obj.remove("x_note");
            obj.remove("expected_digest_sha256");
        }
        serde_json::to_string(&v).unwrap()
    }
    #[test]
    fn real_positive_fixture_deserializes_and_validates_cleanly() {
        // DATUM's mirror validate() is intentionally self-contained (no
        // DGraph/DContinuum/DForward cross-reference -- see this file's own
        // `validate` doc, which unlike DServer's does not take those
        // params), so this proves structural validity + digest, not full
        // cross-contract consistency (that remains DServer's job).
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let map: CanonicalDMapV2 = serde_json::from_str(&strip_fixture_bookkeeping(raw))
            .expect("real positive datum.dmap/2 fixture must deserialize in DATUM's mirror");
        assert!(map.validate().is_empty(), "{:?}", map.validate());
    }
    #[test]
    fn digest_matches_dserver_side_computation() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let map: CanonicalDMapV2 = serde_json::from_str(&strip_fixture_bookkeeping(raw)).unwrap();
        assert_eq!(
            map.digest().unwrap(),
            "sha256:b38a2aa111f358cb92fda232d6a5f707f798b6e4dca278626a7e0dd5a7e39851",
            "DATUM mirror digest must equal DServer's independently-computed digest (ADR-0007 mirror parity)"
        );
    }
    #[test]
    fn digest_is_order_independent() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let mut a: CanonicalDMapV2 = serde_json::from_str(&strip_fixture_bookkeeping(raw)).unwrap();
        let mut b = a.clone();
        b.placements.reverse();
        b.diot_placements.reverse();
        assert_eq!(a.digest().unwrap(), b.digest().unwrap());
        a.dmap_id = "different".into();
        assert_ne!(a.digest().unwrap(), b.digest().unwrap());
    }
    #[test]
    fn wrong_schema_is_rejected() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let mut map: CanonicalDMapV2 =
            serde_json::from_str(&strip_fixture_bookkeeping(raw)).unwrap();
        map.schema = "datum.dmap/1".into();
        assert!(map.validate().iter().any(|f| f.code == "schema_invalid"));
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
        assert!(result.is_err());
    }
    #[test]
    fn duplicate_placement_is_rejected() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let mut map: CanonicalDMapV2 =
            serde_json::from_str(&strip_fixture_bookkeeping(raw)).unwrap();
        let dup = map.placements[0].clone();
        map.placements.push(dup);
        assert!(map
            .validate()
            .iter()
            .any(|f| f.code == "duplicate_placement"));
    }
    #[test]
    fn wildcard_channel_is_rejected() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/dmap-v2/dmap.smart-sentinel-lab.json"
        );
        let mut map: CanonicalDMapV2 =
            serde_json::from_str(&strip_fixture_bookkeeping(raw)).unwrap();
        map.dcall_realizations[0].channel = "iot/#".into();
        assert!(map
            .validate()
            .iter()
            .any(|f| f.code == "realization_invalid"));
    }
}
