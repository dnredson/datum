use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::service_artifact_registry::{
    EXECUTION_PINNING_MODE_LEGACY_PARTIAL, EXECUTION_PINNING_MODE_REQUIRED,
};

pub const OPERATIONAL_DGRAPH_SCHEMA_VERSION: &str = "0.1.0";
pub const OPERATIONAL_DGRAPH_BOOTSTRAP_MODE_EMPTY: &str = "empty";
pub const OPERATIONAL_RECONCILER_MODULE_ID: &str = "smartsentinel-reconciler-v1";
pub const OPERATIONAL_RECONCILER_FUNCTION: &str = "reconcile_service";
pub const AUTHORIZATION_SCOPE_PROCESS: &str = "process";
pub const AUTHORIZATION_SCOPE_DURABLE: &str = "durable";
pub const MAX_DURABLE_AUTHORIZATION_SECONDS: u64 = 86_400;
pub const AUTHORIZATION_OPERATION_RECONCILE: &str = "reconcile";
pub const AUTHORIZATION_OPERATION_ROLLBACK: &str = "rollback";

fn default_dgraph_revision() -> u64 {
    1
}

fn default_authorization_scope() -> String {
    AUTHORIZATION_SCOPE_PROCESS.to_string()
}

fn default_authorization_operation() -> String {
    AUTHORIZATION_OPERATION_RECONCILE.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraph {
    pub schema_version: String,
    pub dgraph_id: String,
    #[serde(default = "default_dgraph_revision")]
    pub revision: u64,
    pub project_id: String,
    pub nodes: Vec<OperationalNodeDesiredState>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalNodeDesiredState {
    pub node_id: String,
    #[serde(default)]
    pub desired_services: Vec<OperationalDesiredService>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalDesiredService {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
    /// Phase 115C (Phase 115A audit finding): the `application_id` of the
    /// D-Map that placed this exact service here. Previously discarded the
    /// moment `union_project_operational_graph` merged per-application
    /// `ActiveApplicationDeployment` records into this shared, per-project,
    /// per-node structure -- it survived only in that function's untyped
    /// `metadata.applications` provenance array, never on the typed service
    /// record itself, so nothing reading `desired_services` directly (this
    /// object's only real consumers: `ArtifactReconciliationContext`,
    /// `OperationalArtifactAssignment`, and ultimately node-local runtime
    /// state) could recover which application a given `service_id` belonged
    /// to. Empty for a not-yet-rebuilt legacy graph -- `union_project_operational_graph`
    /// always populates this going forward; an empty value here must never
    /// be silently treated as a real application_id, only as "unknown."
    #[serde(default)]
    pub application_id: String,
    /// Phase 115C: the exact accepted D-Map `{id, revision, digest}` this
    /// placement came from, EXACTLY as `ActiveApplicationDeployment` already
    /// carries it -- never independently recomputed, never a new authority.
    /// `None` for a not-yet-rebuilt legacy graph or for a service still
    /// resolved under `PlacementAuthority::LegacyTargetNodes` (Stage 111,
    /// which predates canonical D-Map placement entirely and has no D-Map to
    /// reference).
    #[serde(default)]
    pub dmap_ref: Option<CanonicalContractReferenceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconciliationAuthorizationRequest {
    pub dgraph_id: String,
    #[serde(default)]
    pub dgraph_revision: Option<u64>,
    pub authorized_by: String,
    pub allow_host_mutation: bool,
    #[serde(default = "default_authorization_operation")]
    pub operation: String,
    #[serde(default)]
    pub persistence_scope: Option<String>,
    #[serde(default)]
    pub valid_for_seconds: Option<u64>,
}

// `Eq` (not just `PartialEq`) was dropped: `snapshot` holds
// `ArtifactReconciliationAssignment`, which is not (and need not be) `Eq`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalReconciliationAuthorization {
    pub authorization_id: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    /// The active Resource Registry binding at issuance. DServer verifies
    /// this against the CURRENTLY active binding when SERVING a context
    /// (`get_operational_reconciliation_context`). The agent deserializes
    /// this field but does not independently re-verify it (review finding
    /// P1-6, v4) -- it instead verifies `resource_id` below, locally. See
    /// `resource_id`'s doc comment and
    /// `DATUM::agent::stage107a_enforcement::check_resource_identity` for
    /// the precise boundary and the bounded `binding_id`-staleness
    /// consequence this leaves for an already-delivered lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    /// The `ResourceRoleBindingRecord.resource_id` active for `node_id` at
    /// issuance (review finding P1-1). Previously this authorization bound
    /// `binding_id` but not `resource_id`, and the agent's own copy of this
    /// type did not even deserialize `binding_id`, so neither side could
    /// independently verify the resource identity the authorization was
    /// meant for -- the contract silently relied on DServer having already
    /// filtered by binding when the context was served. The agent now
    /// verifies this against its own bootstrap-resolved `resource_id`
    /// before any mutation (`BLOCKED_WRONG_RESOURCE_IDENTITY`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
    #[serde(default = "default_authorization_operation")]
    pub operation: String,
    #[serde(default = "default_authorization_scope")]
    pub persistence_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_utc: Option<String>,
    /// Per-desired-service artifact binding (Stage 107A, ADR-0011 §2 / D-7a):
    /// the exact `{artifact_id, execution_generation,
    /// execution_projection_digest}` this authorization's immutable snapshot
    /// grants. Empty for an authorization issued before Stage 107A (or before
    /// artifacts could be resolved) -- such an authorization is never valid
    /// for a governed host mutation (`artifact_bindings_cover` returns
    /// false), only for non-mutating uses.
    #[serde(default)]
    pub artifact_bindings: Vec<ArtifactBindingV1>,
    /// `execution_pinning_mode` bound at issuance (Stage 107A, N-2). Empty
    /// only for a pre-Stage-107A authorization.
    #[serde(default)]
    pub execution_pinning_mode: String,
    /// THE IMMUTABLE SNAPSHOT (review finding P0-1). The exact resolved
    /// artifact assignments for every desired service on this node, AS THEY
    /// EXISTED at the instant this authorization was issued. This is what
    /// makes Option A's "one exact immutable execution snapshot" claim
    /// (ADR-0012) actually true: while this authorization is attached and
    /// valid, the reconciliation context MUST be built from THIS field, not
    /// by re-resolving the (possibly since-mutated) current artifact
    /// registry. A later content-changing PUT to any of these artifacts
    /// changes their `execution_generation` / registry content, but does not
    /// retroactively alter this already-issued snapshot -- it only makes a
    /// *new* context fetch or *new* authorization see the new content.
    /// Empty for a pre-Stage-107A authorization (never sufficient to serve a
    /// snapshot-backed context; callers fall back to current-registry
    /// resolution, matching the pre-Stage-107A/"proposal preview" behaviour).
    #[serde(default)]
    pub snapshot: Vec<crate::core::artifact_reconciliation_context::ArtifactReconciliationAssignment>,
}

/// One service's exact `{artifact_id, execution_generation,
/// execution_projection_digest}` binding within an
/// `OperationalReconciliationAuthorization`'s immutable snapshot (Stage 107A,
/// ADR-0011 §2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactBindingV1 {
    pub service_id: String,
    pub artifact_id: String,
    pub execution_generation: u64,
    pub execution_projection_digest: String,
}

/// DC-4 (ADR-0011 §3, review finding P0-2 [v4]): `required` mode refuses a
/// **durable** authorization unless the artifact registry and the durable
/// operational-authorization state are backed by the exact same control-state
/// SQLite database file. A `process`-scope authorization never touches
/// durable storage, so this check does not apply to it (always eligible).
/// `legacy_partial` is unaffected by this function -- callers only invoke it
/// for `required`.
///
/// PREVIOUS BUG (v3 review, fixed here): this used to be a hardcoded `false`
/// for every durable authorization, justified by a comment claiming the
/// operational-authorization domain is "ALWAYS a separate JSON file". That
/// claim was wrong: `dserver/src/core/operational_dgraph_persistence.rs`
/// (`load_operational_dgraph_persisted_state` /
/// `persist_operational_dgraph_state`, via `load_domain_json` /
/// `persist_domain_json(..., DOMAIN_OPERATIONAL_DGRAPH, ...)`) uses
/// `configured_control_state_db_path()` -- the SAME env-var-resolved path
/// `ServiceArtifactRegistry::load`'s `default_control_state_db_path()` uses
/// -- whenever `DATUM_CONTROL_STATE_DB_PATH` is set, and only falls back to
/// the legacy JSON file when it is not. A deployment with that variable set
/// therefore DOES share one control-state SQLite database across both
/// domains today, and a hardcoded `false` refused it incorrectly.
///
/// This function is now a pure comparison of the two domains' ACTUALLY
/// RESOLVED backend identities, so it reflects reality regardless of how
/// either domain is configured: eligible only when both paths are known
/// (`Some`) and identical. Two `None`s (both JSON-backed, or an in-memory
/// test registry) and two different `Some` paths are both ineligible, along
/// with a `Some`/`None` mix (SQLite one side, JSON the other).
pub fn required_mode_persistence_eligible(
    persistence_scope: &str,
    artifact_registry_backend_path: Option<&std::path::Path>,
    operational_authorization_backend_path: Option<&std::path::Path>,
) -> bool {
    if persistence_scope != AUTHORIZATION_SCOPE_DURABLE {
        return true;
    }
    matches!(
        (artifact_registry_backend_path, operational_authorization_backend_path),
        (Some(a), Some(b)) if a == b
    )
}

pub const BLOCKED_REQUIRED_PERSISTENCE: &str = "blocked_required_persistence";

/// Governed host mutation (M1-M8, ADR-0012 point 9) requires a **finite**
/// lease (ADR-0012, design-review correction 1 / "point 3"): a process-scope
/// authorization (`expires_at_utc = None`) can never satisfy the Option-A
/// bounded-lease guarantee, because an executing agent has no way to observe
/// that its originating DServer process has ended. This is deliberately
/// distinct from `authorization_is_valid_for_context`, which still accepts a
/// live process-scope authorization for non-mutating purposes.
pub fn authorization_has_finite_lease(authorization: &OperationalReconciliationAuthorization) -> bool {
    authorization.expires_at_utc.is_some()
}

/// Does `authorization.artifact_bindings` bind every `(service_id,
/// artifact_id, execution_generation, execution_projection_digest)` in
/// `assignments`, with no extra or missing bindings? Stage 107A generation /
/// projection-digest gate (ADR-0011 §2), evaluated identically by DServer
/// (for authorize-time / context-time defense in depth) and the agent (at
/// each governed mutation boundary, against its own recomputed digest).
pub fn artifact_bindings_match<'a>(
    authorization: &OperationalReconciliationAuthorization,
    expected: impl Iterator<Item = (&'a str, &'a str, u64, &'a str)>,
) -> bool {
    let mut remaining: std::collections::BTreeMap<&str, &ArtifactBindingV1> = authorization
        .artifact_bindings
        .iter()
        .map(|binding| (binding.service_id.as_str(), binding))
        .collect();
    let mut count = 0usize;
    for (service_id, artifact_id, generation, digest) in expected {
        count += 1;
        match remaining.remove(service_id) {
            Some(binding)
                if crate::core::jcs::is_canonical_digest_format(digest)
                    && binding.artifact_id == artifact_id
                    && binding.execution_generation == generation
                    && binding.execution_projection_digest == digest => {}
            _ => return false,
        }
    }
    remaining.is_empty() && count == authorization.artifact_bindings.len()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconcilerDescriptor {
    pub module_id: String,
    pub function_name: String,
    pub abi: String,
    pub host_imports_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraphSlice {
    pub schema_version: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub project_id: String,
    pub node_id: String,
    pub desired_services: Vec<OperationalDesiredService>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<OperationalReconciliationAuthorization>,
    pub reconciler: OperationalReconcilerDescriptor,
}

fn is_empty_bootstrap_graph(graph: &OperationalDGraph) -> bool {
    graph
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("bootstrap_mode"))
        .and_then(Value::as_str)
        == Some(OPERATIONAL_DGRAPH_BOOTSTRAP_MODE_EMPTY)
}

pub fn validate_operational_dgraph(graph: &OperationalDGraph) -> Result<(), String> {
    if graph.schema_version.trim() != OPERATIONAL_DGRAPH_SCHEMA_VERSION {
        return Err(format!(
            "operational D-Graph schema_version must be {}",
            OPERATIONAL_DGRAPH_SCHEMA_VERSION
        ));
    }
    required("dgraph_id", &graph.dgraph_id)?;
    required("project_id", &graph.project_id)?;
    let empty_bootstrap = is_empty_bootstrap_graph(graph);
    if graph.revision == 0 && !empty_bootstrap {
        return Err("operational D-Graph revision must be greater than zero".to_string());
    }
    if graph.nodes.is_empty() && !empty_bootstrap {
        return Err("operational D-Graph must contain at least one node".to_string());
    }
    if empty_bootstrap && (graph.revision != 0 || !graph.nodes.is_empty()) {
        return Err(
            "empty operational D-Graph bootstrap must use revision 0 and contain no nodes"
                .to_string(),
        );
    }

    let mut seen_nodes = HashSet::new();
    for node in &graph.nodes {
        required("node_id", &node.node_id)?;
        if !seen_nodes.insert(node.node_id.trim().to_string()) {
            return Err(format!("duplicate operational D-Graph node_id: {}", node.node_id));
        }

        let mut seen_services = HashSet::new();
        for service in &node.desired_services {
            validate_desired_service(service)?;
            if !seen_services.insert(service.service_id.trim().to_string()) {
                return Err(format!(
                    "duplicate desired service '{}' for node '{}'",
                    service.service_id, node.node_id
                ));
            }
        }
    }
    Ok(())
}

pub fn build_operational_slice(
    graph: &OperationalDGraph,
    node_id: &str,
    authorization: Option<&OperationalReconciliationAuthorization>,
) -> Result<OperationalDGraphSlice, String> {
    validate_operational_dgraph(graph)?;
    let node_id = node_id.trim();
    let node = graph
        .nodes
        .iter()
        .find(|node| node.node_id.trim() == node_id)
        .ok_or_else(|| format!("node '{}' is not present in operational D-Graph", node_id))?;

    let authorization = authorization
        .filter(|auth| {
            auth.project_id == graph.project_id
                && auth.dgraph_id == graph.dgraph_id
                && auth.dgraph_revision == graph.revision
                && auth.node_id == node_id
        })
        .cloned();

    Ok(OperationalDGraphSlice {
        schema_version: graph.schema_version.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        project_id: graph.project_id.clone(),
        node_id: node.node_id.clone(),
        desired_services: node.desired_services.clone(),
        authorization,
        reconciler: OperationalReconcilerDescriptor {
            module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
            function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
            abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
            host_imports_allowed: false,
        },
    })
}

/// Builds and binds the Stage 107A immutable execution snapshot (ADR-0011 §2,
/// ADR-0012 Option A). `artifact_bindings` and `execution_pinning_mode` are
/// resolved by the caller (`authorize_operational_reconciliation`, which has
/// access to the artifact registry) from the current desired-service/artifact
/// join for `node_id`, using the same resolution rules as
/// `build_artifact_reconciliation_context`.
#[allow(clippy::too_many_arguments)]
pub fn build_reconciliation_authorization(
    graph: &OperationalDGraph,
    node_id: &str,
    binding_id: &str,
    resource_id: &str,
    request: &OperationalReconciliationAuthorizationRequest,
    now: DateTime<Utc>,
    artifact_bindings: Vec<ArtifactBindingV1>,
    execution_pinning_mode: &str,
    snapshot: Vec<crate::core::artifact_reconciliation_context::ArtifactReconciliationAssignment>,
) -> Result<OperationalReconciliationAuthorization, String> {
    if !matches!(
        execution_pinning_mode,
        EXECUTION_PINNING_MODE_LEGACY_PARTIAL | EXECUTION_PINNING_MODE_REQUIRED
    ) {
        return Err(format!(
            "unsupported execution_pinning_mode '{}'; expected '{}' or '{}'",
            execution_pinning_mode, EXECUTION_PINNING_MODE_LEGACY_PARTIAL, EXECUTION_PINNING_MODE_REQUIRED
        ));
    }
    validate_operational_dgraph(graph)?;
    let node_id = node_id.trim();
    let binding_id = binding_id.trim();
    if !graph.nodes.iter().any(|node| node.node_id.trim() == node_id) {
        return Err(format!("node '{}' is not present in operational D-Graph", node_id));
    }
    required("binding_id", binding_id)?;
    if request.dgraph_id.trim() != graph.dgraph_id {
        return Err(format!(
            "authorization dgraph_id '{}' does not match active operational D-Graph '{}'",
            request.dgraph_id, graph.dgraph_id
        ));
    }
    if let Some(requested_revision) = request.dgraph_revision {
        if requested_revision != graph.revision {
            return Err(format!(
                "authorization dgraph_revision '{}' does not match active operational D-Graph revision '{}'",
                requested_revision, graph.revision
            ));
        }
    }
    required("authorized_by", &request.authorized_by)?;
    if !request.allow_host_mutation {
        return Err(
            "operational reconciliation authorization must explicitly set allow_host_mutation=true"
                .to_string(),
        );
    }

    let operation = request.operation.trim();
    if !matches!(operation, AUTHORIZATION_OPERATION_RECONCILE | AUTHORIZATION_OPERATION_ROLLBACK) {
        return Err(format!(
            "unsupported operational authorization operation '{}'; expected '{}' or '{}'",
            operation, AUTHORIZATION_OPERATION_RECONCILE, AUTHORIZATION_OPERATION_ROLLBACK
        ));
    }

    let persistence_scope = request
        .persistence_scope
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(AUTHORIZATION_SCOPE_PROCESS);

    let expires_at_utc = match persistence_scope {
        AUTHORIZATION_SCOPE_PROCESS => {
            if request.valid_for_seconds.is_some() {
                return Err(
                    "valid_for_seconds is only supported for persistence_scope=durable"
                        .to_string(),
                );
            }
            None
        }
        AUTHORIZATION_SCOPE_DURABLE => {
            let seconds = request.valid_for_seconds.ok_or_else(|| {
                "durable operational authorization requires valid_for_seconds".to_string()
            })?;
            if seconds == 0 || seconds > MAX_DURABLE_AUTHORIZATION_SECONDS {
                return Err(format!(
                    "durable operational authorization valid_for_seconds must be between 1 and {}",
                    MAX_DURABLE_AUTHORIZATION_SECONDS
                ));
            }
            let seconds_i64 = i64::try_from(seconds)
                .map_err(|_| "durable authorization duration is out of range".to_string())?;
            Some((now + Duration::seconds(seconds_i64)).to_rfc3339())
        }
        other => {
            return Err(format!(
                "unsupported operational authorization persistence_scope '{}'; expected '{}' or '{}'",
                other, AUTHORIZATION_SCOPE_PROCESS, AUTHORIZATION_SCOPE_DURABLE
            ));
        }
    };

    Ok(OperationalReconciliationAuthorization {
        authorization_id: format!("opauth:{}", Uuid::new_v4()),
        project_id: graph.project_id.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        node_id: node_id.to_string(),
        binding_id: Some(binding_id.to_string()),
        resource_id: Some(resource_id.to_string()),
        authorized_by: request.authorized_by.trim().to_string(),
        authorized_at_utc: now.to_rfc3339(),
        allow_host_mutation: true,
        operation: operation.to_string(),
        persistence_scope: persistence_scope.to_string(),
        expires_at_utc,
        artifact_bindings,
        execution_pinning_mode: execution_pinning_mode.to_string(),
        snapshot,
    })
}

pub fn authorization_is_valid_for_context(
    authorization: &OperationalReconciliationAuthorization,
    graph: &OperationalDGraph,
    node_id: &str,
    binding_id: &str,
    now: DateTime<Utc>,
) -> bool {
    if !authorization.allow_host_mutation
        || authorization.project_id != graph.project_id
        || authorization.dgraph_id != graph.dgraph_id
        || authorization.dgraph_revision != graph.revision
        || authorization.node_id != node_id.trim()
        || authorization.binding_id.as_deref() != Some(binding_id.trim())
    {
        return false;
    }

    match authorization.expires_at_utc.as_deref() {
        Some(expires) => DateTime::parse_from_rfc3339(expires)
            .map(|expires| expires.with_timezone(&Utc) > now)
            .unwrap_or(false),
        None => authorization.persistence_scope == AUTHORIZATION_SCOPE_PROCESS,
    }
}

fn validate_desired_service(service: &OperationalDesiredService) -> Result<(), String> {
    required("service_id", &service.service_id)?;
    // Stage 108 (ADR-0015, P0-4 correction): this was previously the third
    // independent `== "container"` gate found in this codebase (the other
    // two live in `DATUM/src/agent/operational_reconciliation.rs`), hard-
    // coded to accept only `"container"`. Runtime-kind parsing now
    // recognises both canonical wire values; anything else still fails
    // closed, exactly as before.
    if crate::core::runtime_kind::RuntimeKind::parse(&service.runtime).is_err() {
        return Err(format!(
            "operational service '{}' uses unsupported runtime '{}'; expected 'container' or 'native_process'",
            service.service_id, service.runtime
        ));
    }
    if !matches!(
        service.desired_state.trim().to_ascii_lowercase().as_str(),
        "running" | "completed"
    ) {
        return Err(format!(
            "operational service '{}' uses unsupported desired_state '{}'; expected running or completed",
            service.service_id, service.desired_state
        ));
    }
    Ok(())
}

fn required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("operational D-Graph {} must not be empty", label))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: "dgraph:o2-mqtt-role-1".to_string(),
            revision: 1,
            project_id: "monitor-f49c8".to_string(),
            nodes: vec![OperationalNodeDesiredState {
                node_id: "fog-01".to_string(),
                desired_services: vec![OperationalDesiredService {
                    service_id: "mqtt".to_string(),
                    runtime: "container".to_string(),
                    desired_state: "running".to_string(),
                    application_id: String::new(),
                    dmap_ref: None,
                }],
            }],
            metadata: None,
        }
    }

    #[test]
    fn mqtt_only_node_slice_is_valid() {
        let graph = graph();
        validate_operational_dgraph(&graph).unwrap();
        let slice = build_operational_slice(&graph, "fog-01", None).unwrap();
        assert_eq!(slice.dgraph_revision, 1);
        assert_eq!(slice.desired_services.len(), 1);
        assert_eq!(slice.desired_services[0].service_id, "mqtt");
        assert_eq!(slice.reconciler.module_id, OPERATIONAL_RECONCILER_MODULE_ID);
        assert!(!slice.reconciler.host_imports_allowed);
    }

    #[test]
    fn unsupported_runtime_is_rejected() {
        let mut graph = graph();
        graph.nodes[0].desired_services[0].runtime = "process".to_string();
        assert!(validate_operational_dgraph(&graph).is_err());
    }

    #[test]
    fn completed_one_shot_service_is_valid() {
        let mut graph = graph();
        graph.nodes[0].desired_services[0].desired_state = "completed".to_string();

        validate_operational_dgraph(&graph).unwrap();
        let slice = build_operational_slice(&graph, "fog-01", None).unwrap();

        assert_eq!(slice.desired_services[0].desired_state, "completed");
    }

    #[test]
    fn unsupported_desired_state_is_rejected() {
        let mut graph = graph();
        graph.nodes[0].desired_services[0].desired_state = "paused".to_string();

        assert!(validate_operational_dgraph(&graph).is_err());
    }

    #[test]
    fn zero_revision_is_rejected() {
        let mut graph = graph();
        graph.revision = 0;
        assert!(validate_operational_dgraph(&graph).is_err());
    }

    #[test]
    fn process_authorization_is_binding_and_revision_scoped() {
        let graph = graph();
        let request = OperationalReconciliationAuthorizationRequest {
            dgraph_id: graph.dgraph_id.clone(),
            dgraph_revision: Some(graph.revision),
            authorized_by: "operator:test".to_string(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            persistence_scope: None,
            valid_for_seconds: None,
        };
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 18, 45, 0).unwrap();
        let auth = build_reconciliation_authorization(
            &graph,
            "fog-01",
            "binding:test",
            "resource:test",
            &request,
            now,
            Vec::new(),
            EXECUTION_PINNING_MODE_REQUIRED,
            Vec::new(),
        )
        .unwrap();
        assert!(authorization_is_valid_for_context(
            &auth,
            &graph,
            "fog-01",
            "binding:test",
            now
        ));
        assert!(!authorization_is_valid_for_context(
            &auth,
            &graph,
            "fog-01",
            "binding:other",
            now
        ));
    }

    #[test]
    fn durable_authorization_requires_finite_lease_and_expires() {
        let graph = graph();
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 18, 45, 0).unwrap();
        let request = OperationalReconciliationAuthorizationRequest {
            dgraph_id: graph.dgraph_id.clone(),
            dgraph_revision: Some(graph.revision),
            authorized_by: "operator:test".to_string(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            persistence_scope: Some(AUTHORIZATION_SCOPE_DURABLE.to_string()),
            valid_for_seconds: Some(60),
        };
        let auth = build_reconciliation_authorization(
            &graph,
            "fog-01",
            "binding:test",
            "resource:test",
            &request,
            now,
            Vec::new(),
            EXECUTION_PINNING_MODE_REQUIRED,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(auth.persistence_scope, AUTHORIZATION_SCOPE_DURABLE);
        assert!(auth.expires_at_utc.is_some());
        assert!(authorization_is_valid_for_context(
            &auth,
            &graph,
            "fog-01",
            "binding:test",
            now + Duration::seconds(59)
        ));
        assert!(!authorization_is_valid_for_context(
            &auth,
            &graph,
            "fog-01",
            "binding:test",
            now + Duration::seconds(60)
        ));
    }
    #[test]
    fn stage106_empty_project_bootstrap_graph_is_valid_and_inert() {
        let graph = OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: "dgraph:project-new".to_string(),
            revision: 0,
            project_id: "project-new".to_string(),
            nodes: Vec::new(),
            metadata: Some(serde_json::json!({
                "bootstrap_mode": OPERATIONAL_DGRAPH_BOOTSTRAP_MODE_EMPTY,
                "host_mutation_allowed": false,
            })),
        };
        assert!(validate_operational_dgraph(&graph).is_ok());

        let mut invalid = graph.clone();
        invalid.revision = 1;
        assert!(validate_operational_dgraph(&invalid).is_err());

        let mut invalid = graph;
        invalid.metadata = None;
        assert!(validate_operational_dgraph(&invalid).is_err());
    }

    // ---- Stage 107A: finite lease + artifact bindings --------------------- //

    fn binding(service_id: &str, artifact_id: &str, generation: u64, digest: &str) -> ArtifactBindingV1 {
        ArtifactBindingV1 {
            service_id: service_id.to_string(),
            artifact_id: artifact_id.to_string(),
            execution_generation: generation,
            execution_projection_digest: digest.to_string(),
        }
    }

    fn base_auth() -> OperationalReconciliationAuthorization {
        OperationalReconciliationAuthorization {
            authorization_id: "opauth:test".to_string(),
            project_id: "p".to_string(),
            dgraph_id: "d".to_string(),
            dgraph_revision: 1,
            node_id: "n".to_string(),
            binding_id: None,
            resource_id: None,
            authorized_by: "operator".to_string(),
            authorized_at_utc: "2026-01-01T00:00:00Z".to_string(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            persistence_scope: AUTHORIZATION_SCOPE_PROCESS.to_string(),
            expires_at_utc: None,
            artifact_bindings: vec![binding("mqtt", "mqtt", 3, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")],
            execution_pinning_mode: "required".to_string(),
            snapshot: Vec::new(),
        }
    }

    #[test]
    fn process_scope_authorization_never_has_a_finite_lease() {
        assert!(!authorization_has_finite_lease(&base_auth()));
    }

    #[test]
    fn durable_authorization_with_expiry_has_a_finite_lease() {
        let mut auth = base_auth();
        auth.persistence_scope = AUTHORIZATION_SCOPE_DURABLE.to_string();
        auth.expires_at_utc = Some("2026-01-01T01:00:00Z".to_string());
        assert!(authorization_has_finite_lease(&auth));
    }

    #[test]
    fn artifact_bindings_match_exact_set() {
        let auth = base_auth();
        assert!(artifact_bindings_match(
            &auth,
            [("mqtt", "mqtt", 3u64, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")].into_iter()
        ));
    }

    #[test]
    fn artifact_bindings_reject_generation_mismatch() {
        let auth = base_auth();
        assert!(!artifact_bindings_match(
            &auth,
            [("mqtt", "mqtt", 4u64, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")].into_iter()
        ));
    }

    #[test]
    fn artifact_bindings_reject_digest_mismatch() {
        let auth = base_auth();
        assert!(!artifact_bindings_match(
            &auth,
            [("mqtt", "mqtt", 3u64, "different")].into_iter()
        ));
    }

    #[test]
    fn artifact_bindings_reject_missing_service() {
        let auth = base_auth();
        assert!(!artifact_bindings_match(
            &auth,
            [("mqtt", "mqtt", 3u64, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"), ("api", "api", 1u64, "x")].into_iter()
        ));
    }

    #[test]
    fn artifact_bindings_reject_extra_binding_not_expected() {
        let mut auth = base_auth();
        auth.artifact_bindings.push(binding("api", "api", 1, "x"));
        assert!(!artifact_bindings_match(
            &auth,
            [("mqtt", "mqtt", 3u64, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")].into_iter()
        ));
    }

    #[test]
    fn empty_artifact_bindings_never_match_any_expectation() {
        let mut auth = base_auth();
        auth.artifact_bindings.clear();
        assert!(!artifact_bindings_match(
            &auth,
            [("mqtt", "mqtt", 3u64, "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")].into_iter()
        ));
    }

    // ---- P0-2 (v4 review): required-mode split-persistence refusal --------
    // -- (DC-4), now a pure comparison of resolved backend paths ---------- //
    #[test]
    fn required_mode_refuses_durable_authorization_when_backends_are_both_json() {
        // Two `None`s: both domains fell back to their legacy JSON file.
        assert!(!required_mode_persistence_eligible(
            AUTHORIZATION_SCOPE_DURABLE,
            None,
            None,
        ));
    }

    #[test]
    fn required_mode_refuses_durable_authorization_when_backends_differ() {
        let registry_path = std::path::PathBuf::from("/data/control-state-a.sqlite");
        let operational_path = std::path::PathBuf::from("/data/control-state-b.sqlite");
        assert!(!required_mode_persistence_eligible(
            AUTHORIZATION_SCOPE_DURABLE,
            Some(&registry_path),
            Some(&operational_path),
        ));
    }

    #[test]
    fn required_mode_refuses_durable_authorization_when_only_one_side_is_sqlite() {
        let registry_path = std::path::PathBuf::from("/data/control-state.sqlite");
        assert!(!required_mode_persistence_eligible(
            AUTHORIZATION_SCOPE_DURABLE,
            Some(&registry_path),
            None,
        ));
    }

    #[test]
    fn required_mode_accepts_durable_authorization_when_backends_are_the_same_sqlite_file() {
        let shared_path = std::path::PathBuf::from("/data/control-state.sqlite");
        assert!(required_mode_persistence_eligible(
            AUTHORIZATION_SCOPE_DURABLE,
            Some(&shared_path),
            Some(&shared_path),
        ));
    }

    #[test]
    fn required_mode_check_does_not_apply_to_process_scope() {
        assert!(required_mode_persistence_eligible(
            AUTHORIZATION_SCOPE_PROCESS,
            None,
            None,
        ));
    }
}
