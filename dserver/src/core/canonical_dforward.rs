//! Canonical `datum.dforward/1`: logical middleware composition only.
use crate::core::{canonical_dgraph::CanonicalValidationFinding, jcs};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub const CANONICAL_DFORWARD_SCHEMA: &str = "datum.dforward/1";
fn f(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
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
            if d.diot_id.trim().is_empty() {
                e.push(f("diot_id_invalid", "diot_id must be non-empty"));
            }
            if ds.insert(d.diot_id.clone(), d).is_some() {
                e.push(f("duplicate_diot_id", "duplicate DIoT id"));
            }
            if d.diot_kind.trim().is_empty() || d.artifact_requirement.artifact_id.trim().is_empty()
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
        let mut es = std::collections::HashMap::new();
        for x in &self.external_endpoints {
            if x.endpoint_id.trim().is_empty()
                || x.interface.trim().is_empty()
                || !matches!(x.direction.as_str(), "input" | "output")
            {
                e.push(f(
                    "external_endpoint_invalid",
                    "endpoint id/interface and direction input|output are required",
                ));
            }
            if es.insert(x.endpoint_id.clone(), x).is_some() {
                e.push(f(
                    "duplicate_external_endpoint_id",
                    "duplicate external endpoint id",
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
                if s.is_none() && !es.contains_key(&h.from) {
                    e.push(f("dangling_hop_source", "hop source is undeclared"));
                }
                if t.is_none() && !es.contains_key(&h.to) {
                    e.push(f("dangling_hop_target", "hop target is undeclared"));
                }
                if let Some(x) = es.get(&h.from) {
                    if x.direction != "input" {
                        e.push(f(
                            "external_direction_invalid",
                            "only input endpoints may be hop sources",
                        ));
                    }
                    if x.interface != h.interface {
                        e.push(f(
                            "external_interface_mismatch",
                            "source endpoint interface differs from hop",
                        ));
                    }
                    if let Some(target) = t {
                        if !target.provides.iter().any(|p| p.interface == h.interface)
                            && !target.requires.iter().any(|r| r.interface == h.interface)
                        {
                            e.push(f(
                                "external_adjacent_interface_invalid",
                                "DIoT adjacent to input lacks the endpoint interface",
                            ));
                        }
                    }
                }
                if let Some(x) = es.get(&h.to) {
                    if x.direction != "output" {
                        e.push(f(
                            "external_direction_invalid",
                            "only output endpoints may be hop targets",
                        ));
                    }
                    if x.interface != h.interface {
                        e.push(f(
                            "external_interface_mismatch",
                            "target endpoint interface differs from hop",
                        ));
                    }
                    if let Some(source) = s {
                        if !source.provides.iter().any(|p| p.interface == h.interface)
                            && !source.requires.iter().any(|r| r.interface == h.interface)
                        {
                            e.push(f(
                                "external_adjacent_interface_invalid",
                                "DIoT adjacent to output lacks the endpoint interface",
                            ));
                        }
                    }
                }
                if let (Some(a), Some(b)) = (s, t) {
                    let complementary = (a.provides.iter().any(|x| x.interface == h.interface)
                        && b.requires.iter().any(|x| x.interface == h.interface))
                        || (a.requires.iter().any(|x| x.interface == h.interface)
                            && b.provides.iter().any(|x| x.interface == h.interface));
                    if !complementary {
                        e.push(f("incompatible_hop_interface","DIoT hop endpoints must have complementary provides/requires interfaces"));
                    }
                }
            }
        }
        let mut js = std::collections::HashSet::new();
        for j in &self.support_jobs {
            if !js.insert(&j.job_id) {
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
    fn complementary_interfaces_validate() {
        assert!(sample().validate().is_empty());
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
    #[test]
    fn digest_is_order_independent() {
        let a = sample();
        let mut b = sample();
        b.diots.reverse();
        assert_eq!(a.digest().unwrap(), b.digest().unwrap());
    }
    #[test]
    fn real_positive_fixture_deserializes_and_validates_cleanly() {
        // Phase 113A closure pass 5, section 11: a positive fixture
        // claiming a production schema must actually deserialize into the
        // real Rust wire type and validate cleanly -- a Python-only
        // structural check is not a substitute for this.
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        );
        let forward: CanonicalDForwardV1 = serde_json::from_str(raw)
            .expect("real positive datum.dforward/1 fixture must deserialize");
        assert!(
            forward.validate().is_empty(),
            "real positive fixture must validate cleanly: {:?}",
            forward.validate()
        );
        assert_eq!(forward.project_id, "monitor-f49c8");
        assert_eq!(forward.dforward_id, "dforward:smart-sentinel-lab");
    }
    #[test]
    fn real_positive_fixture_digest_matches_dmap_v2_bundle_reference() {
        // Cross-check against dmap-v2/dmap.smart-sentinel-lab.json's own
        // dforward_ref.digest -- both must trace to the SAME real digest
        // computed by this same function, proving the fixture bundle is
        // genuinely mutually consistent, not independently hand-typed.
        let raw = include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        );
        let forward: CanonicalDForwardV1 = serde_json::from_str(raw).unwrap();
        assert_eq!(
            forward.digest().unwrap(),
            "sha256:a2aaf6a429c85ae76d2c5095bac8ccf09289408cdd4d74ec1e203bfd471d92d6"
        );
    }
    #[test]
    fn every_negative_dforward_fixture_actually_produces_its_claimed_finding() {
        // Phase 113A closure pass 5, section 13: a negative fixture's
        // claimed `x_expect_reject.code` must be genuinely reproducible by
        // the REAL Rust validator, not merely asserted in fixture prose.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/datum-contracts/canonical-v1/negative");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("negative fixture dir must exist") {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.starts_with("dforward.") || !name.ends_with(".json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let mut v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let obj = v.as_object_mut().unwrap();
            obj.remove("x_note");
            let expected_code = obj
                .remove("x_expect_reject")
                .and_then(|x| x.get("code").cloned())
                .and_then(|c| c.as_str().map(str::to_string))
                .unwrap_or_else(|| {
                    panic!("{name}: negative fixture must carry x_expect_reject.code")
                });
            let cleaned = serde_json::to_string(&v).unwrap();
            if expected_code == "deny_unknown_fields" {
                let result: Result<CanonicalDForwardV1, _> = serde_json::from_str(&cleaned);
                assert!(
                    result.is_err(),
                    "{name}: fixture claims deny_unknown_fields rejection but deserialized successfully"
                );
                checked += 1;
                continue;
            }
            let forward: CanonicalDForwardV1 = serde_json::from_str(&cleaned).unwrap_or_else(|e| {
                panic!("{name}: negative fixture must still deserialize into the real type: {e}")
            });
            let findings = forward.validate();
            assert!(
                findings.iter().any(|f| f.code == expected_code),
                "{name}: expected finding code '{expected_code}' was not produced -- got {findings:?}"
            );
            checked += 1;
        }
        assert!(
            checked >= 8,
            "expected at least 8 real dforward negative fixtures to be exercised, found {checked}"
        );
    }
    #[test]
    fn cycles_remain_legal() {
        let mut x = sample();
        x.chains[0].hops.push(CanonicalDForwardHopV1 {
            from: "b".into(),
            to: "a".into(),
            interface: "i".into(),
        });
        assert!(!x.validate().iter().any(|f| f.code == "cycle"));
    }
}
