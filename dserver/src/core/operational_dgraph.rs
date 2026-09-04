use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const OPERATIONAL_DGRAPH_SCHEMA_VERSION: &str = "0.1.0";
pub const OPERATIONAL_RECONCILER_MODULE_ID: &str = "smartsentinel-reconciler-v1";
pub const OPERATIONAL_RECONCILER_FUNCTION: &str = "reconcile_service";

fn default_dgraph_revision() -> u64 {
    1
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconciliationAuthorization {
    pub authorization_id: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
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
    request: &OperationalReconciliationAuthorizationRequest,
    now: DateTime<Utc>,
) -> Result<OperationalReconciliationAuthorization, String> {
    validate_operational_dgraph(graph)?;
    let node_id = node_id.trim();
    if !graph.nodes.iter().any(|node| node.node_id.trim() == node_id) {
        return Err(format!("node '{}' is not present in operational D-Graph", node_id));
    }
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

    Ok(OperationalReconciliationAuthorization {
        authorization_id: format!("opauth:{}", Uuid::new_v4()),
        project_id: graph.project_id.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        node_id: node_id.to_string(),
        authorized_by: request.authorized_by.trim().to_string(),
        authorized_at_utc: now.to_rfc3339(),
        allow_host_mutation: true,
    })
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
    fn authorization_is_bound_to_active_graph_revision_and_node() {
        let graph = graph();
        let request = OperationalReconciliationAuthorizationRequest {
            dgraph_id: graph.dgraph_id.clone(),
            dgraph_revision: Some(graph.revision),
            authorized_by: "operator:test".to_string(),
            allow_host_mutation: true,
        };
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 18, 45, 0).unwrap();
        let auth = build_reconciliation_authorization(&graph, "fog-01", &request, now).unwrap();
        let slice = build_operational_slice(&graph, "fog-01", Some(&auth)).unwrap();
        assert_eq!(slice.authorization.unwrap().dgraph_revision, graph.revision);
    }
}
