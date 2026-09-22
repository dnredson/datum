//! D-Call operational D-Node transport endpoint registry (ADR-0022, Phase
//! 112). Purely OPERATIONAL routing configuration -- how to reach a D-Node
//! over the network for cross-D-Node D-Call delivery -- deliberately kept
//! separate from `CanonicalDNodeV1`/D-Continuum structural authority, which
//! stays host-independent by design (Stage 110/Phase 110 architectural
//! invariant, unchanged and un-reopened here). An endpoint declaration:
//!
//! - MUST reference an already-declared structural `CanonicalDNodeV1`
//!   (`dnode_registry`) -- an endpoint for an unknown node is rejected;
//! - carries NO capacity/platform/ABI/stage information of its own (that
//!   remains exclusively `dnode_registry`'s concern);
//! - is looked up by DServer only at D-Call grant issuance/redemption time,
//!   never folded into any canonical digest (`recompute_dgraph_digest`,
//!   `recompute_dmap_digest`, `recompute_dserv_artifact_digest` never read
//!   this registry -- see `endpoint-canonical-contamination-audit.txt`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::dnode_registry::DNodeRegistryStateV1;
use crate::core::jcs;
use crate::storage::control_state::{
    load_domain_json, persist_domain_json, DOMAIN_DCALL_ENDPOINT_REGISTRY,
};

pub const DCALL_ENDPOINT_REGISTRY_STATE_VERSION: &str = "dcall_endpoint_registry_v1";

/// v0.1 supports exactly one concrete transport (ADR-0022 section 17): a
/// bare, unauthenticated HTTP listener suitable only for the existing
/// trusted/lab network boundary. Restricting the allowed value here (rather
/// than accepting an arbitrary string) is deliberate: it is the cheapest
/// way to guarantee "unsupported route transport" (test item 61 in the
/// original spec) actually fails closed instead of silently accepting a
/// transport nothing implements.
pub const TRANSPORT_HTTP: &str = "http";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

fn length_prefixed_component(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

pub fn endpoint_key(node_id: &str, transport: &str) -> String {
    format!(
        "{}{}",
        length_prefixed_component(node_id),
        length_prefixed_component(transport)
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DNodeTransportEndpointV1 {
    pub node_id: String,
    pub transport: String,
    pub endpoint_id: String,
    pub revision: u64,
    pub base_url: String,
    /// Canonical JCS+SHA-256 digest (ADR-0006) of this descriptor's own
    /// content (everything except this field itself) -- lets a listener/
    /// caller detect a stale cached copy without re-fetching, and lets
    /// reload integrity validation (`validate_persisted_dcall_endpoint_registry_integrity`)
    /// detect corruption of the persisted record itself.
    pub descriptor_digest: String,
}

impl DNodeTransportEndpointV1 {
    fn digest_input(&self) -> serde_json::Value {
        serde_json::json!({
            "node_id": self.node_id,
            "transport": self.transport,
            "endpoint_id": self.endpoint_id,
            "revision": self.revision,
            "base_url": self.base_url,
        })
    }
}

pub fn recompute_endpoint_digest(endpoint: &DNodeTransportEndpointV1) -> Result<String, String> {
    jcs::canonical_digest_sha256_hex(&endpoint.digest_input()).map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DCallEndpointRegistryStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub endpoints: BTreeMap<String, DNodeTransportEndpointV1>,
}

impl DCallEndpointRegistryStateV1 {
    pub fn empty() -> Self {
        Self {
            state_version: DCALL_ENDPOINT_REGISTRY_STATE_VERSION.to_string(),
            endpoints: BTreeMap::new(),
        }
    }

    pub fn get(&self, node_id: &str, transport: &str) -> Option<&DNodeTransportEndpointV1> {
        self.endpoints.get(&endpoint_key(node_id, transport))
    }
}

/// Validates a `base_url` per ADR-0022 v0.1 rules: an explicit scheme
/// restricted to the transports v0.1 actually implements, NO embedded
/// userinfo (`user:pass@host`) -- credentials never belong in a routing
/// URL -- and a parseable host. Returns the parsed URL's scheme so the
/// caller can cross-check it against `transport`.
fn validate_base_url(base_url: &str) -> Result<String, CanonicalValidationFinding> {
    let parsed = url::Url::parse(base_url).map_err(|error| {
        finding(
            "dcall_endpoint_url_invalid",
            format!("base_url '{base_url}' is not a valid URL: {error}"),
        )
    })?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(finding(
            "dcall_endpoint_scheme_unsupported",
            format!(
                "base_url scheme '{}' is not a v0.1-supported D-Call transport scheme",
                parsed.scheme()
            ),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(finding(
            "dcall_endpoint_url_userinfo_rejected",
            "base_url must not contain embedded userinfo (credentials never belong in a routing URL)".to_string(),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(finding(
            "dcall_endpoint_url_invalid",
            format!("base_url '{base_url}' has no host"),
        ));
    }
    if parsed.query().is_some() {
        return Err(finding(
            "dcall_endpoint_url_query_rejected",
            "base_url must not contain a query component",
        ));
    }
    if parsed.fragment().is_some() {
        return Err(finding(
            "dcall_endpoint_url_fragment_rejected",
            "base_url must not contain a fragment component",
        ));
    }
    Ok(parsed.scheme().to_string())
}

#[derive(Debug)]
pub enum DeclareEndpointError {
    Invalid(CanonicalValidationFinding),
    NodeUnknown(String),
}

/// Declares or updates one `(node_id, transport)` endpoint. Unlike
/// `dcode_registry`'s immutable-conflict-only precedent, an endpoint
/// declaration is explicitly mutable (ADR-0022 section 8: "updates
/// increment revision") -- a D-Node's operational address genuinely can
/// change (redeploy, IP change) without that being a canonical-authority
/// event. Every successful declare/update strictly increments `revision`
/// (starting at 1), and the returned digest is always freshly recomputed --
/// never trusted from caller input.
pub fn declare_endpoint(
    state: &mut DCallEndpointRegistryStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    node_id: &str,
    transport: &str,
    base_url: &str,
) -> Result<DNodeTransportEndpointV1, DeclareEndpointError> {
    if !dnode_registry_state.nodes.contains_key(node_id) {
        return Err(DeclareEndpointError::NodeUnknown(format!(
            "node '{node_id}' is not a currently declared structural D-Node -- an endpoint cannot be declared for an unregistered node"
        )));
    }
    if transport != TRANSPORT_HTTP {
        return Err(DeclareEndpointError::Invalid(finding(
            "dcall_endpoint_transport_unsupported",
            format!("transport '{transport}' is not a v0.1-supported D-Call transport (only '{TRANSPORT_HTTP}' is implemented)"),
        )));
    }
    let scheme = validate_base_url(base_url).map_err(DeclareEndpointError::Invalid)?;
    if scheme != "http" {
        // v0.1's one concrete transport is bare HTTP; https is accepted by
        // validate_base_url as a generically well-formed scheme but is not
        // yet an implemented transport, so reject it the same way an
        // unsupported `transport` string is rejected.
        return Err(DeclareEndpointError::Invalid(finding(
            "dcall_endpoint_scheme_unsupported",
            format!("base_url scheme '{scheme}' is not implemented in v0.1 (only 'http' is)"),
        )));
    }

    let key = endpoint_key(node_id, transport);
    let next_revision = state
        .endpoints
        .get(&key)
        .map(|existing| existing.revision + 1)
        .unwrap_or(1);
    let endpoint_id = format!("dcall-endpoint:{node_id}:{transport}");
    let mut endpoint = DNodeTransportEndpointV1 {
        node_id: node_id.to_string(),
        transport: transport.to_string(),
        endpoint_id,
        revision: next_revision,
        base_url: base_url.to_string(),
        descriptor_digest: String::new(),
    };
    endpoint.descriptor_digest = recompute_endpoint_digest(&endpoint).map_err(|error| {
        DeclareEndpointError::Invalid(finding("dcall_endpoint_digest_unrecomputable", error))
    })?;
    state.endpoints.insert(key, endpoint.clone());
    Ok(endpoint)
}

/// Fail-closed relational integrity check, invoked from `load_state`.
/// Mirrors `dnode_registry`/`dcode_registry`'s precedent: map-key
/// consistency, each stored endpoint's digest re-verified, revision must be
/// >= 1 (there is no "revision 0" state for an endpoint -- unlike the
/// D-Node registry, an EMPTY endpoint map is the only valid "nothing
/// declared yet" state), no silent normalization/repair of corruption.
pub fn validate_persisted_dcall_endpoint_registry_integrity(
    state: &DCallEndpointRegistryStateV1,
) -> Result<(), String> {
    if state.state_version != DCALL_ENDPOINT_REGISTRY_STATE_VERSION {
        return Err(format!(
            "unsupported dcall endpoint registry state_version '{}', expected '{DCALL_ENDPOINT_REGISTRY_STATE_VERSION}'",
            state.state_version
        ));
    }
    for (key, endpoint) in &state.endpoints {
        let expected_key = endpoint_key(&endpoint.node_id, &endpoint.transport);
        if *key != expected_key {
            return Err(format!(
                "dcall endpoint stored under key '{key}' does not match endpoint_key('{}', '{}') = '{expected_key}'",
                endpoint.node_id, endpoint.transport
            ));
        }
        if endpoint.revision < 1 {
            return Err(format!(
                "dcall endpoint '{key}' has revision {} but revision must be >= 1",
                endpoint.revision
            ));
        }
        // I7: re-run FULL structural endpoint validation at reload, not
        // merely recompute the digest -- a persisted record whose base_url
        // was corrupted to an unsupported scheme/userinfo/transport
        // combination, with its digest recomputed to match, would otherwise
        // pass silently.
        if endpoint.transport != TRANSPORT_HTTP {
            return Err(format!(
                "dcall endpoint '{key}' has unsupported persisted transport '{}'",
                endpoint.transport
            ));
        }
        let scheme = validate_base_url(&endpoint.base_url).map_err(|f| {
            format!(
                "dcall endpoint '{key}' fails structural re-validation: {}",
                f.message
            )
        })?;
        if scheme != "http" {
            return Err(format!(
                "dcall endpoint '{key}' has unsupported persisted scheme '{scheme}'"
            ));
        }
        let expected_digest = recompute_endpoint_digest(endpoint).map_err(|error| {
            format!("dcall endpoint '{key}' descriptor digest could not be recomputed: {error}")
        })?;
        if expected_digest != endpoint.descriptor_digest {
            return Err(format!(
                "dcall endpoint '{key}' persisted descriptor_digest '{}' does not match recomputed '{expected_digest}' -- persisted state is corrupt or was tampered with",
                endpoint.descriptor_digest
            ));
        }
    }
    Ok(())
}

pub fn load_state(db_path: &std::path::Path) -> Result<DCallEndpointRegistryStateV1, String> {
    match load_domain_json::<DCallEndpointRegistryStateV1>(db_path, DOMAIN_DCALL_ENDPOINT_REGISTRY)?
    {
        Some(state) => {
            validate_persisted_dcall_endpoint_registry_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DCallEndpointRegistryStateV1::empty()),
    }
}

pub fn persist_state(
    db_path: &std::path::Path,
    state: &DCallEndpointRegistryStateV1,
) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DCALL_ENDPOINT_REGISTRY, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
        CanonicalNodePlatformV1,
    };
    use crate::core::dnode_registry::declare_dnode;

    fn dnode_state_with(node_id: &str) -> DNodeRegistryStateV1 {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(
            &mut state,
            CanonicalDNodeV1 {
                node_id: node_id.to_string(),
                platform: CanonicalNodePlatformV1 {
                    os: "linux".to_string(),
                    architecture: "x86_64".to_string(),
                },
                capacity: CanonicalNodeCapacityV1 {
                    cpu_cores: 4,
                    memory_bytes: 4_294_967_296,
                },
                capabilities: CanonicalNodeCapabilitiesV1 {
                    dnode_abi_versions: vec!["datum-dnode/0".to_string()],
                },
            },
        )
        .unwrap();
        state
    }

    #[test]
    fn declaring_endpoint_for_unknown_node_is_rejected() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let dnodes = DNodeRegistryStateV1::empty();
        let error = declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://127.0.0.1:9100",
        )
        .unwrap_err();
        assert!(matches!(error, DeclareEndpointError::NodeUnknown(_)));
    }

    #[test]
    fn valid_endpoint_declaration_succeeds_at_revision_one() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let dnodes = dnode_state_with("fog-01");
        let endpoint = declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://127.0.0.1:9100",
        )
        .unwrap();
        assert_eq!(endpoint.revision, 1);
        assert!(!endpoint.descriptor_digest.is_empty());
    }

    #[test]
    fn redeclaring_endpoint_increments_revision() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let dnodes = dnode_state_with("fog-01");
        declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://127.0.0.1:9100",
        )
        .unwrap();
        let updated = declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://127.0.0.1:9200",
        )
        .unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.base_url, "http://127.0.0.1:9200");
    }

    #[test]
    fn userinfo_in_url_is_rejected() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let dnodes = dnode_state_with("fog-01");
        let error = declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://user:pass@127.0.0.1:9100",
        )
        .unwrap_err();
        assert!(
            matches!(error, DeclareEndpointError::Invalid(f) if f.code == "dcall_endpoint_url_userinfo_rejected")
        );
    }

    #[test]
    fn query_and_fragment_urls_are_rejected_without_registry_mutation() {
        for (url, code) in [
            ("http://host/base?x=1", "dcall_endpoint_url_query_rejected"),
            ("http://host/base?", "dcall_endpoint_url_query_rejected"),
            (
                "http://host/#fragment",
                "dcall_endpoint_url_fragment_rejected",
            ),
            ("http://host/#", "dcall_endpoint_url_fragment_rejected"),
        ] {
            let mut state = DCallEndpointRegistryStateV1::empty();
            let dnodes = dnode_state_with("fog-01");
            let error =
                declare_endpoint(&mut state, &dnodes, "fog-01", TRANSPORT_HTTP, url).unwrap_err();
            assert!(matches!(error, DeclareEndpointError::Invalid(f) if f.code == code));
            assert!(state.endpoints.is_empty());
        }
    }

    #[test]
    fn path_prefix_remains_valid() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let value = declare_endpoint(
            &mut state,
            &dnode_state_with("fog-01"),
            "fog-01",
            TRANSPORT_HTTP,
            "http://host/prefix",
        )
        .unwrap();
        assert_eq!(value.base_url, "http://host/prefix");
    }

    #[test]
    fn unsupported_scheme_is_rejected() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let dnodes = dnode_state_with("fog-01");
        let error = declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "ftp://127.0.0.1:9100",
        )
        .unwrap_err();
        assert!(
            matches!(error, DeclareEndpointError::Invalid(f) if f.code == "dcall_endpoint_scheme_unsupported")
        );
    }

    #[test]
    fn unsupported_transport_string_is_rejected() {
        let mut state = DCallEndpointRegistryStateV1::empty();
        let dnodes = dnode_state_with("fog-01");
        let error = declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            "mqtt",
            "http://127.0.0.1:9100",
        )
        .unwrap_err();
        assert!(
            matches!(error, DeclareEndpointError::Invalid(f) if f.code == "dcall_endpoint_transport_unsupported")
        );
    }

    #[test]
    fn persisted_reload_roundtrips() {
        let dir = tempfile_dir();
        let db_path = dir.join("control-state.sqlite3");
        let dnodes = dnode_state_with("fog-01");
        let mut state = load_state(&db_path).unwrap();
        declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://127.0.0.1:9100",
        )
        .unwrap();
        persist_state(&db_path, &state).unwrap();
        let reloaded = load_state(&db_path).unwrap();
        assert_eq!(reloaded, state);
    }

    #[test]
    fn tampered_descriptor_digest_fails_reload() {
        let dir = tempfile_dir();
        let db_path = dir.join("control-state.sqlite3");
        let dnodes = dnode_state_with("fog-01");
        let mut state = load_state(&db_path).unwrap();
        declare_endpoint(
            &mut state,
            &dnodes,
            "fog-01",
            TRANSPORT_HTTP,
            "http://127.0.0.1:9100",
        )
        .unwrap();
        let key = endpoint_key("fog-01", TRANSPORT_HTTP);
        state.endpoints.get_mut(&key).unwrap().base_url = "http://127.0.0.1:9999".to_string();
        persist_state(&db_path, &state).unwrap();
        let error = load_state(&db_path).unwrap_err();
        assert!(error.contains("does not match recomputed"));
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("dcall-endpoint-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
