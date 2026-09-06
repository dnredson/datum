use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const OPERATIONAL_DGRAPH_SCHEMA_VERSION: &str = "0.1.0";
pub const OPERATIONAL_RECONCILER_MODULE_ID: &str = "smartsentinel-reconciler-v1";
pub const OPERATIONAL_RECONCILER_FUNCTION: &str = "reconcile_service";
pub const AUTHORIZATION_SCOPE_PROCESS: &str = "process";
pub const AUTHORIZATION_SCOPE_DURABLE: &str = "durable";
pub const MAX_DURABLE_AUTHORIZATION_SECONDS: u64 = 86_400;

fn default_dgraph_revision() -> u64 {
    1
}

fn default_authorization_scope() -> String {
    AUTHORIZATION_SCOPE_PROCESS.to_string()
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconciliationAuthorizationRequest {
    pub dgraph_id: String,
    #[serde(default)]
    pub dgraph_revision: Option<u64>,
    pub authorized_by: String,
    pub allow_host_mutation: bool,
    #[serde(default)]
    pub persistence_scope: Option<String>,
    #[serde(default)]
    pub valid_for_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconciliationAuthorization {
    pub authorization_id: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
    #[serde(default = "default_authorization_scope")]
    pub persistence_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_utc: Option<String>,
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

pub fn validate_operational_dgraph(graph: &OperationalDGraph) -> Result<(), String> {
    if graph.schema_version.trim() != OPERATIONAL_DGRAPH_SCHEMA_VERSION {
        return Err(format!(
            "operational D-Graph schema_version must be {}",
            OPERATIONAL_DGRAPH_SCHEMA_VERSION
        ));
    }
    required("dgraph_id", &graph.dgraph_id)?;
    required("project_id", &graph.project_id)?;
    if graph.revision == 0 {
        return Err("operational D-Graph revision must be greater than zero".to_string());
    }
    if graph.nodes.is_empty() {
        return Err("operational D-Graph must contain at least one node".to_string());
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

pub fn build_reconciliation_authorization(
    graph: &OperationalDGraph,
    node_id: &str,
    binding_id: &str,
    request: &OperationalReconciliationAuthorizationRequest,
    now: DateTime<Utc>,
) -> Result<OperationalReconciliationAuthorization, String> {
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
        authorized_by: request.authorized_by.trim().to_string(),
        authorized_at_utc: now.to_rfc3339(),
        allow_host_mutation: true,
        persistence_scope: persistence_scope.to_string(),
        expires_at_utc,
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
    if !service.runtime.trim().eq_ignore_ascii_case("container") {
        return Err(format!(
            "operational service '{}' uses unsupported runtime '{}'; O2 vertical slice supports only container",
            service.service_id, service.runtime
        ));
    }
    if !service.desired_state.trim().eq_ignore_ascii_case("running") {
        return Err(format!(
            "operational service '{}' uses unsupported desired_state '{}'; O2 vertical slice supports only running",
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
            persistence_scope: None,
            valid_for_seconds: None,
        };
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 18, 45, 0).unwrap();
        let auth = build_reconciliation_authorization(
            &graph,
            "fog-01",
            "binding:test",
            &request,
            now,
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
            persistence_scope: Some(AUTHORIZATION_SCOPE_DURABLE.to_string()),
            valid_for_seconds: Some(60),
        };
        let auth = build_reconciliation_authorization(
            &graph,
            "fog-01",
            "binding:test",
            &request,
            now,
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
}
