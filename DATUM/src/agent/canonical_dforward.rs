//! SmartSentinel mirror of canonical `datum.dforward/1`.
use crate::agent::{canonical_dgraph::CanonicalValidationFinding, jcs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const CANONICAL_DFORWARD_SCHEMA: &str = "datum.dforward/1";
fn f(c: &'static str, m: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code: c,
        message: m.into(),
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDForwardV1 {
    pub schema: String,
    pub project_id: String,
    pub dforward_id: String,
    pub revision: u64,
    #[serde(default)]
    pub diots: Vec<CanonicalDIoTV1>,
    #[serde(default)]
    pub external_endpoints: Vec<CanonicalExternalEndpointV1>,
    #[serde(default)]
    pub chains: Vec<CanonicalDForwardChainV1>,
    #[serde(default)]
    pub support_jobs: Vec<CanonicalDForwardSupportJobV1>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDIoTV1 {
    pub diot_id: String,
    pub diot_kind: String,
    pub artifact_requirement: CanonicalArtifactRequirementV1,
    #[serde(default)]
    pub provides: Vec<CanonicalDForwardInterfaceV1>,
    #[serde(default)]
    pub requires: Vec<CanonicalDForwardRequirementV1>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalArtifactRequirementV1 {
    pub artifact_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDForwardInterfaceV1 {
    pub interface: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDForwardRequirementV1 {
    pub interface: String,
    pub scope: DForwardRequirementScope,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DForwardRequirementScope {
    SameElement,
    AnyElement,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalExternalEndpointV1 {
    pub endpoint_id: String,
    pub direction: String,
    pub interface: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDForwardChainV1 {
    pub chain_id: String,
    #[serde(default)]
    pub hops: Vec<CanonicalDForwardHopV1>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDForwardHopV1 {
    pub from: String,
    pub to: String,
    pub interface: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDForwardSupportJobV1 {
    pub job_id: String,
    pub artifact_requirement: CanonicalArtifactRequirementV1,
    pub desired_state: String,
}
impl CanonicalDForwardV1 {
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut e = Vec::new();
        if self.schema != CANONICAL_DFORWARD_SCHEMA {
            e.push(f("schema_invalid", "schema must be datum.dforward/1"));
        }
        if self.project_id.trim().is_empty() {
            e.push(f("missing_field", "project_id must be non-empty"));
        }
        if self.dforward_id.trim().is_empty() {
            e.push(f("missing_field", "dforward_id must be non-empty"));
        }
        if self.revision < 1 {
            e.push(f("revision_invalid", "revision must be >= 1"));
        }
        let mut ds = std::collections::HashMap::new();
        for d in &self.diots {
            if ds.insert(d.diot_id.clone(), d).is_some() {
                e.push(f("duplicate_diot_id", "duplicate DIoT id"));
            }
            if d.diot_id.trim().is_empty()
                || d.diot_kind.trim().is_empty()
                || d.artifact_requirement.artifact_id.trim().is_empty()
            {
                e.push(f(
                    "diot_invalid",
                    "DIoT kind and artifact_id must be non-empty",
                ));
            }
            for p in &d.provides {
                if p.interface.trim().is_empty() {
                    e.push(f(
                        "interface_invalid",
                        "provided interface must be non-empty",
                    ));
                }
            }
            for r in &d.requires {
                if r.interface.trim().is_empty() {
                    e.push(f(
                        "interface_invalid",
                        "required interface must be non-empty",
                    ));
                }
            }
        }
        let mut ex = std::collections::HashMap::new();
        for x in &self.external_endpoints {
            if ex.insert(x.endpoint_id.clone(), x).is_some() {
                e.push(f(
                    "duplicate_external_endpoint_id",
                    "duplicate external endpoint id",
                ));
            }
            if x.endpoint_id.trim().is_empty()
                || x.interface.trim().is_empty()
                || !matches!(x.direction.as_str(), "input" | "output")
            {
                e.push(f(
                    "external_endpoint_invalid",
                    "endpoint id/interface and direction input|output are required",
                ));
            }
        }
        let mut cs = std::collections::HashMap::new();
        for c in &self.chains {
            if c.chain_id.trim().is_empty() {
                e.push(f("chain_id_invalid", "chain_id must be non-empty"));
            }
            if cs.insert(c.chain_id.clone(), c).is_some() {
                e.push(f("duplicate_chain_id", "duplicate chain id"));
            }
            for h in &c.hops {
                if h.interface.trim().is_empty() {
                    e.push(f(
                        "hop_interface_invalid",
                        "hop interface must be non-empty",
                    ));
                }
                let s = ds.get(&h.from);
                let t = ds.get(&h.to);
                if s.is_none() && !ex.contains_key(&h.from) {
                    e.push(f("dangling_hop_source", "hop source is undeclared"));
                }
                if t.is_none() && !ex.contains_key(&h.to) {
                    e.push(f("dangling_hop_target", "hop target is undeclared"));
                }
                if let Some(x) = ex.get(&h.from) {
                    if x.direction != "input" || x.interface != h.interface {
                        e.push(f(
                            "external_direction_invalid",
                            "input endpoint source must match interface",
                        ));
                    }
                    if let Some(target) = t {
                        if !target.provides.iter().any(|p| p.interface == h.interface)
                            && !target.requires.iter().any(|r| r.interface == h.interface)
                        {
                            e.push(f(
                                "external_adjacent_interface_invalid",
                                "adjacent DIoT lacks input interface",
                            ));
                        }
                    }
                }
                if let Some(x) = ex.get(&h.to) {
                    if x.direction != "output" || x.interface != h.interface {
                        e.push(f(
                            "external_direction_invalid",
                            "output endpoint target must match interface",
                        ));
                    }
                    if let Some(source) = s {
                        if !source.provides.iter().any(|p| p.interface == h.interface)
                            && !source.requires.iter().any(|r| r.interface == h.interface)
                        {
                            e.push(f(
                                "external_adjacent_interface_invalid",
                                "adjacent DIoT lacks output interface",
                            ));
                        }
                    }
                }
                if let (Some(a), Some(b)) = (s, t) {
                    let ok = (a.provides.iter().any(|x| x.interface == h.interface)
                        && b.requires.iter().any(|x| x.interface == h.interface))
                        || (a.requires.iter().any(|x| x.interface == h.interface)
                            && b.provides.iter().any(|x| x.interface == h.interface));
                    if !ok {
                        e.push(f(
                            "incompatible_hop_interface",
                            "DIoT hop endpoints must have complementary interfaces",
                        ));
                    }
                }
            }
        }
        let mut jobs = std::collections::HashSet::new();
        for j in &self.support_jobs {
            if !jobs.insert(&j.job_id) {
                e.push(f("duplicate_support_job", "duplicate support job"));
            }
            if j.job_id.trim().is_empty()
                || j.desired_state.trim().is_empty()
                || j.artifact_requirement.artifact_id.trim().is_empty()
            {
                e.push(f("support_job_invalid", "support job is incomplete"));
            }
        }
        e
    }
    pub fn digest(&self) -> Result<String, String> {
        let mut x = self.clone();
        x.diots.sort_by(|a, b| a.diot_id.cmp(&b.diot_id));
        x.external_endpoints
            .sort_by(|a, b| a.endpoint_id.cmp(&b.endpoint_id));
        x.chains.sort_by(|a, b| a.chain_id.cmp(&b.chain_id));
        x.support_jobs.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        let v: Value = serde_json::to_value(x).map_err(|e| e.to_string())?;
        jcs::canonical_digest_sha256_hex(&v).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Phase 113A closure pass 5, section 12: DATUM-side mirror tests for
    /// `CanonicalDForwardV1`, cross-checked against DServer's real fixture
    /// and its independently-verified real digest wherever the check is
    /// locally computable without server-only registry/acceptance
    /// authority (which stays DServer-only per the task's own scope note).
    #[test]
    fn real_positive_fixture_deserializes_and_validates_cleanly() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        );
        let forward: CanonicalDForwardV1 = serde_json::from_str(raw)
            .expect("real positive datum.dforward/1 fixture must deserialize in DATUM's mirror");
        assert!(forward.validate().is_empty(), "{:?}", forward.validate());
    }
    #[test]
    fn digest_matches_dserver_side_computation() {
        // Cross-crate parity vector: the SAME real fixture, deserialized by
        // DATUM's independently-mirrored type, must produce the exact same
        // digest DServer's own `canonical_dforward.rs` test independently
        // computed and asserted for this file.
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        );
        let forward: CanonicalDForwardV1 = serde_json::from_str(raw).unwrap();
        assert_eq!(
            forward.digest().unwrap(),
            "sha256:a2aaf6a429c85ae76d2c5095bac8ccf09289408cdd4d74ec1e203bfd471d92d6",
            "DATUM mirror digest must equal DServer's independently-computed digest for the identical fixture (ADR-0007 mirror parity)"
        );
    }
    fn sample() -> CanonicalDForwardV1 {
        CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.into(),
            project_id: "p".into(),
            dforward_id: "f".into(),
            revision: 1,
            diots: vec![
                CanonicalDIoTV1 {
                    diot_id: "a".into(),
                    diot_kind: "bridge".into(),
                    artifact_requirement: CanonicalArtifactRequirementV1 {
                        artifact_id: "x".into(),
                    },
                    provides: vec![],
                    requires: vec![CanonicalDForwardRequirementV1 {
                        interface: "i".into(),
                        scope: DForwardRequirementScope::AnyElement,
                    }],
                },
                CanonicalDIoTV1 {
                    diot_id: "b".into(),
                    diot_kind: "broker".into(),
                    artifact_requirement: CanonicalArtifactRequirementV1 {
                        artifact_id: "y".into(),
                    },
                    provides: vec![CanonicalDForwardInterfaceV1 {
                        interface: "i".into(),
                    }],
                    requires: vec![],
                },
            ],
            external_endpoints: vec![],
            chains: vec![CanonicalDForwardChainV1 {
                chain_id: "c".into(),
                hops: vec![CanonicalDForwardHopV1 {
                    from: "a".into(),
                    to: "b".into(),
                    interface: "i".into(),
                }],
            }],
            support_jobs: vec![],
        }
    }
    #[test]
    fn digest_is_order_independent() {
        let a = sample();
        let mut b = sample();
        b.diots.reverse();
        assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    }
    #[test]
    fn wrong_schema_is_rejected() {
        let mut x = sample();
        x.schema = "datum.x-fixture.vs1-dforward/1".into();
        assert!(x.validate().iter().any(|f| f.code == "schema_invalid"));
    }
    #[test]
    fn unknown_field_is_rejected_by_deny_unknown_fields() {
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        );
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("rogue_field".into(), Value::Bool(true));
        let text = serde_json::to_string(&v).unwrap();
        let result: Result<CanonicalDForwardV1, _> = serde_json::from_str(&text);
        assert!(result.is_err());
    }
    #[test]
    fn duplicate_diot_id_is_rejected() {
        let mut x = sample();
        x.diots[1].diot_id = "a".into();
        assert!(x.validate().iter().any(|f| f.code == "duplicate_diot_id"));
    }
    #[test]
    fn incompatible_interfaces_reject() {
        let mut x = sample();
        x.diots[1].provides.clear();
        assert!(x
            .validate()
            .iter()
            .any(|f| f.code == "incompatible_hop_interface"));
    }
}
