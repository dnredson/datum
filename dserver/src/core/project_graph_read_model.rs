//! Phase 131: bounded logical structure from accepted immutable DGraph snapshots.
//! No registry, placement, runtime state, artifact configuration or graph writes.
use super::canonical_dgraph::CanonicalDGraphV1;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_GRAPH_APPLICATIONS: usize = 100;
pub const MAX_GRAPH_NODES: usize = 2000;
pub const MAX_GRAPH_EDGES: usize = 4000;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphApplication {
    pub application_id: String,
    pub dgraph_id: String,
    pub revision: u64,
    pub digest: String,
    pub acceptance_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Application,
    Dserv,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: GraphNodeKind,
    pub canonical_id: String,
    pub application_id: String,
    pub parent_id: Option<String>,
    pub execution_abi: Option<String>,
    pub cpu_cores: Option<u32>,
    // Decimal string preserves exact canonical u64 values in JavaScript.
    pub memory_bytes: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeKind {
    Dcall,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphEdge {
    pub id: String,
    pub kind: GraphEdgeKind,
    pub canonical_id: String,
    pub application_id: String,
    pub source: String,
    pub target: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphCoverage {
    pub scope: &'static str,
    pub completeness: &'static str,
    pub active_application_count: usize,
    pub included_application_count: usize,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphContent {
    pub coverage: GraphCoverage,
    pub applications: Vec<GraphApplication>,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}
// JSON tuple is a collision-free opaque presentation key. Never parsed as authority.
fn key(application: &str, graph: &str, kind: &str, id: &str) -> String {
    serde_json::to_string(&(application, graph, kind, id)).expect("string tuple serialization")
}
pub fn build(
    snapshots: Vec<(CanonicalDGraphV1, String, Option<String>)>,
) -> Result<GraphContent, String> {
    let mut ordered = BTreeMap::new();
    for (graph, digest, acceptance) in snapshots {
        if !graph.validate().is_empty() {
            return Err("invalid accepted logical snapshot".into());
        }
        if ordered
            .insert(graph.application_id.clone(), (graph, digest, acceptance))
            .is_some()
        {
            return Err("duplicate accepted application graph".into());
        }
    }
    let total = ordered.len();
    let mut result = GraphContent {
        coverage: GraphCoverage {
            scope: "active_accepted_applications_only",
            completeness: "complete",
            active_application_count: total,
            included_application_count: 0,
        },
        applications: vec![],
        nodes: vec![],
        edges: vec![],
    };
    for (application, (graph, digest, acceptance_id)) in ordered {
        // Atomic per application: never emit dangling edges or silently partial graphs.
        if result.applications.len() == MAX_GRAPH_APPLICATIONS
            || result.nodes.len() + graph.services.len() + 1 > MAX_GRAPH_NODES
            || result.edges.len() + graph.calls.len() > MAX_GRAPH_EDGES
        {
            result.coverage.completeness = "partial";
            break;
        }
        let parent = key(&application, &graph.dgraph_id, "application", &application);
        result.applications.push(GraphApplication {
            application_id: application.clone(),
            dgraph_id: graph.dgraph_id.clone(),
            revision: graph.revision,
            digest,
            acceptance_id,
        });
        result.nodes.push(GraphNode {
            id: parent.clone(),
            kind: GraphNodeKind::Application,
            canonical_id: application.clone(),
            application_id: application.clone(),
            parent_id: None,
            execution_abi: None,
            cpu_cores: None,
            memory_bytes: None,
        });
        for service in graph.services {
            result.nodes.push(GraphNode {
                id: key(&application, &graph.dgraph_id, "dserv", &service.service_id),
                kind: GraphNodeKind::Dserv,
                canonical_id: service.service_id,
                application_id: application.clone(),
                parent_id: Some(parent.clone()),
                execution_abi: Some(service.execution_requirement.dnode_abi),
                cpu_cores: Some(service.resource_requirement.cpu_cores),
                memory_bytes: Some(service.resource_requirement.memory_bytes.to_string()),
            });
        }
        for call in graph.calls {
            result.edges.push(GraphEdge {
                id: key(&application, &graph.dgraph_id, "dcall", &call.call_id),
                kind: GraphEdgeKind::Dcall,
                canonical_id: call.call_id,
                application_id: application.clone(),
                source: key(
                    &application,
                    &graph.dgraph_id,
                    "dserv",
                    &call.source_service_id,
                ),
                target: key(
                    &application,
                    &graph.dgraph_id,
                    "dserv",
                    &call.target_service_id,
                ),
            });
        }
    }
    result.coverage.included_application_count = result.applications.len();
    result.nodes.sort_by(|a, b| a.id.cmp(&b.id));
    result.edges.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: BTreeSet<_> = result.nodes.iter().map(|n| &n.id).collect();
    if result
        .edges
        .iter()
        .any(|e| !ids.contains(&e.source) || !ids.contains(&e.target))
    {
        return Err("dangling logical edge".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph(app: &str) -> CanonicalDGraphV1 {
        serde_json::from_value(serde_json::json!({"schema":"datum.dgraph/1","application_id":app,"dgraph_id":"logical","revision":3,
            "services":[{"service_id":"a","execution_requirement":{"dnode_abi":"datum-dnode/0"},"resource_requirement":{"cpu_cores":1,"memory_bytes":123}},
                {"service_id":"b","execution_requirement":{"dnode_abi":"datum-dnode/0"},"resource_requirement":{"cpu_cores":1,"memory_bytes":123}}],
            "calls":[{"call_id":"ab","source_service_id":"a","target_service_id":"b"},{"call_id":"ba","source_service_id":"b","target_service_id":"a"},{"call_id":"aa","source_service_id":"a","target_service_id":"a"}]})).unwrap()
    }
    fn sized(app: &str, services: usize, calls: usize) -> CanonicalDGraphV1 {
        let mut g = graph(app);
        let template = g.services[0].clone();
        g.services = (0..services)
            .map(|i| {
                let mut s = template.clone();
                s.service_id = format!("service-{i:04}");
                s
            })
            .collect();
        g.calls = (0..calls)
            .map(|i| super::super::canonical_dgraph::CanonicalDCallV1 {
                call_id: format!("call-{i:04}"),
                source_service_id: "service-0000".into(),
                target_service_id: "service-0000".into(),
            })
            .collect();
        g
    }
    fn snapshot(g: CanonicalDGraphV1) -> (CanonicalDGraphV1, String, Option<String>) {
        (g, "digest".into(), None)
    }
    fn counts(g: &GraphContent, active: usize, included: usize, omitted: usize) {
        assert_eq!(g.coverage.active_application_count, active);
        assert_eq!(g.coverage.included_application_count, included);
        assert_eq!(
            g.coverage.active_application_count - g.coverage.included_application_count,
            omitted
        );
        assert_eq!(g.applications.len(), included);
    }
    #[test]
    fn node_limit_atomically_omits_whole_application_and_stops_before_later_small_one() {
        let result = build(vec![
            snapshot(sized("c", 0, 0)),
            snapshot(sized("b", 2, 1)),
            snapshot(sized("a", 1998, 0)),
        ])
        .unwrap();
        counts(&result, 3, 1, 2);
        assert_eq!(result.coverage.completeness, "partial");
        assert_eq!(result.applications[0].application_id, "a");
        assert_eq!(result.nodes.len(), 1999);
        assert!(result.edges.is_empty());
    }
    #[test]
    fn edge_limit_atomically_omits_whole_application_and_stops_in_sorted_order() {
        let inputs = vec![
            snapshot(sized("c", 0, 0)),
            snapshot(sized("b", 1, 1)),
            snapshot(sized("a", 1, 4000)),
        ];
        let result = build(inputs.clone()).unwrap();
        let mut reversed = inputs;
        reversed.reverse();
        assert_eq!(result, build(reversed).unwrap());
        counts(&result, 3, 1, 2);
        assert_eq!(result.coverage.completeness, "partial");
        assert_eq!(result.nodes.len(), 2);
        assert_eq!(result.edges.len(), 4000);
    }
    #[test]
    fn oversized_first_by_nodes_is_partial_not_valid_empty_source() {
        let result = build(vec![
            snapshot(sized("a", 2000, 0)),
            snapshot(sized("b", 1, 0)),
        ])
        .unwrap();
        counts(&result, 2, 0, 2);
        assert_eq!(result.coverage.completeness, "partial");
        assert!(result.nodes.is_empty() && result.edges.is_empty());
        let empty = build(vec![]).unwrap();
        counts(&empty, 0, 0, 0);
        assert_eq!(empty.coverage.completeness, "complete");
    }
    #[test]
    fn oversized_first_by_edges_is_partial_and_omits_all_later_snapshots() {
        let result = build(vec![
            snapshot(sized("a", 1, 4001)),
            snapshot(sized("b", 1, 0)),
        ])
        .unwrap();
        counts(&result, 2, 0, 2);
        assert_eq!(result.coverage.completeness, "partial");
        assert!(result.nodes.is_empty() && result.edges.is_empty());
    }
    #[test]
    fn cycles_self_calls_identity_and_order_are_preserved() {
        let a = build(vec![
            (graph("z"), "digest".into(), None),
            (graph("a"), "digest".into(), None),
        ])
        .unwrap();
        let mut shuffled = graph("a");
        shuffled.services.reverse();
        shuffled.calls.reverse();
        let b = build(vec![
            (shuffled, "digest".into(), None),
            (graph("z"), "digest".into(), None),
        ])
        .unwrap();
        assert_eq!(a, b);
        assert_eq!(a.nodes.len(), 6);
        assert_eq!(a.edges.len(), 6);
        assert!(a.edges.iter().any(|e| e.source == e.target));
        assert_eq!(a.applications[0].revision, 3);
        assert!(serde_json::to_string(&a).unwrap().find("node_id").is_none());
    }
    #[test]
    fn bounded_complete_application_coverage_not_registry() {
        let inputs = (0..101)
            .map(|i| (graph(&format!("app-{i:03}")), "digest".into(), None))
            .collect();
        let result = build(inputs).unwrap();
        assert_eq!(result.coverage.completeness, "partial");
        assert_eq!(result.coverage.active_application_count, 101);
        counts(&result, 101, 100, 1);
        assert_eq!(result.applications.len(), 100);
        assert_eq!(result.edges.len(), 300);
        assert_eq!(result.coverage.scope, "active_accepted_applications_only");
    }
    #[test]
    fn empty_valid_and_invalid_snapshot_not_silently_empty() {
        assert!(build(vec![]).unwrap().nodes.is_empty());
        let mut invalid = graph("a");
        invalid.calls[0].target_service_id = "missing".into();
        assert!(build(vec![(invalid, "digest".into(), None)]).is_err());
        assert!(build(vec![
            (graph("a"), "d".into(), None),
            (graph("a"), "d".into(), None)
        ])
        .is_err());
    }
}
