use std::{
    collections::{BTreeMap, HashSet, VecDeque},
};

use serde_json::{json, Value};

const DEFAULT_MAX_HOPS: usize = 4;
const MAX_MAX_HOPS: usize = 8;
const MAX_PATHS_PER_PAIR: usize = 3;

#[derive(Debug, Clone)]
struct ObservedEdge {
    from: String,
    to: String,
    relation: Value,
    reachable: Option<bool>,
    rtt_ms: Value,
    telemetry_freshness: String,
    edge_status: String,
}

#[derive(Debug, Clone)]
struct EnumeratedPath {
    nodes: Vec<String>,
    edges: Vec<ObservedEdge>,
}

pub fn plan_alternative_paths(
    observed_dgraph: &Value,
    requested_from: Option<&str>,
    requested_to: Option<&str>,
    requested_max_hops: Option<usize>,
) -> Result<Value, String> {
    let nodes = observed_nodes(observed_dgraph)?;
    let node_ids = nodes
        .iter()
        .map(|node| node.0.clone())
        .collect::<Vec<_>>();
    let node_set = node_ids.iter().cloned().collect::<HashSet<_>>();

    let from = normalize_optional_node(requested_from)?;
    let to = normalize_optional_node(requested_to)?;

    if let Some(node_id) = from.as_deref() {
        if !node_set.contains(node_id) {
            return Err(format!(
                "alternative path source node '{}' is not present in observed D-Graph",
                node_id
            ));
        }
    }
    if let Some(node_id) = to.as_deref() {
        if !node_set.contains(node_id) {
            return Err(format!(
                "alternative path target node '{}' is not present in observed D-Graph",
                node_id
            ));
        }
    }

    let max_hops = requested_max_hops
        .unwrap_or(DEFAULT_MAX_HOPS)
        .clamp(1, MAX_MAX_HOPS);
    let edges = observed_edges(observed_dgraph, &node_set);
    let mut adjacency = BTreeMap::<String, Vec<ObservedEdge>>::new();
    for edge in edges {
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge);
    }
    for outgoing in adjacency.values_mut() {
        outgoing.sort_by(|left, right| {
            left.to
                .cmp(&right.to)
                .then(left.edge_status.cmp(&right.edge_status))
        });
    }

    let pairs = requested_pairs(&node_ids, from.as_deref(), to.as_deref());
    let mut candidates = Vec::new();
    let mut rejected_pairs = Vec::new();

    for (source, target) in pairs {
        let paths = enumerate_simple_paths(&source, &target, &adjacency, max_hops);
        if paths.is_empty() {
            rejected_pairs.push(json!({
                "from": source,
                "to": target,
                "reason": "no_observed_path_within_hop_limit",
                "max_hops": max_hops
            }));
            continue;
        }

        let mut ranked = paths
            .into_iter()
            .map(|path| {
                let assessment = assess_path(&path, &nodes);
                (path, assessment)
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            path_rank(&left.1)
                .cmp(&path_rank(&right.1))
                .then(left.0.edges.len().cmp(&right.0.edges.len()))
                .then(path_key(&left.0).cmp(&path_key(&right.0)))
        });

        for (index, (path, assessment)) in ranked
            .into_iter()
            .take(MAX_PATHS_PER_PAIR)
            .enumerate()
        {
            candidates.push(path_json(
                &source,
                &target,
                index,
                &path,
                &assessment,
            ));
        }
    }

    let eligible_count = candidates
        .iter()
        .filter(|candidate| candidate.get("eligible").and_then(Value::as_bool) == Some(true))
        .count();
    let ineligible_count = candidates.len().saturating_sub(eligible_count);

    Ok(json!({
        "schema_version": "0.1.0",
        "contract_kind": "datum_alternative_path_what_if",
        "project_id": observed_dgraph.get("project_id").cloned().unwrap_or(Value::Null),
        "dgraph_id": observed_dgraph.get("dgraph_id").cloned().unwrap_or(Value::Null),
        "dgraph_revision": observed_dgraph.get("revision").cloned().unwrap_or(Value::Null),
        "observed_path_status": observed_dgraph.get("path_status").cloned().unwrap_or(Value::Null),
        "planner": {
            "kind": "deterministic_bounded_simple_path_enumeration",
            "max_hops": max_hops,
            "max_paths_per_pair": MAX_PATHS_PER_PAIR,
            "execution_performed": false
        },
        "requested_scope": {
            "from": from,
            "to": to
        },
        "candidates": candidates,
        "rejected_pairs": rejected_pairs,
        "summary": {
            "pair_count": requested_pairs(&node_ids, from.as_deref(), to.as_deref()).len(),
            "candidate_count": eligible_count + ineligible_count,
            "eligible_count": eligible_count,
            "ineligible_count": ineligible_count,
            "rejected_pair_count": rejected_pairs.len()
        },
        "what_if": {
            "mode": "proposal_only",
            "changes_applied": false,
            "execution_allowed": false,
            "side_effects_performed": false
        },
        "side_effects_performed": false
    }))
}

fn observed_nodes(observed_dgraph: &Value) -> Result<Vec<(String, bool)>, String> {
    let items = observed_dgraph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "observed D-Graph nodes must be an array".to_string())?;

    let mut seen = HashSet::new();
    let mut nodes = Vec::new();
    for item in items {
        let node_id = item
            .get("node_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "observed D-Graph node_id must be a non-empty string".to_string())?
            .to_string();
        if !seen.insert(node_id.clone()) {
            return Err(format!("duplicate observed D-Graph node '{}'", node_id));
        }

        let observed = item.get("observed");
        let telemetry_fresh = observed
            .and_then(|value| value.get("telemetry_freshness"))
            .and_then(Value::as_str)
            == Some("fresh");
        let failure_healthy = item
            .get("failure_state")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            == Some("healthy");

        nodes.push((node_id, telemetry_fresh && failure_healthy));
    }
    Ok(nodes)
}

fn observed_edges(observed_dgraph: &Value, node_set: &HashSet<String>) -> Vec<ObservedEdge> {
    observed_dgraph
        .get("measured_peer_edges")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let from = item.get("from").and_then(Value::as_str)?.trim().to_string();
            let to = item.get("to").and_then(Value::as_str)?.trim().to_string();
            if from.is_empty() || to.is_empty() || !node_set.contains(&from) || !node_set.contains(&to) {
                return None;
            }

            let reachable = item.get("reachable").and_then(Value::as_bool);
            let telemetry_freshness = item
                .get("telemetry_freshness")
                .and_then(Value::as_str)
                .unwrap_or("missing")
                .to_string();

            Some(ObservedEdge {
                from,
                to,
                relation: item
                    .get("relation")
                    .cloned()
                    .unwrap_or_else(|| json!("peer_connectivity")),
                reachable,
                rtt_ms: item.get("rtt_ms").cloned().unwrap_or(Value::Null),
                edge_status: edge_status(reachable, &telemetry_freshness).to_string(),
                telemetry_freshness,
            })
        })
        .collect()
}

fn normalize_optional_node(value: Option<&str>) -> Result<Option<String>, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.chars().count() > 128 {
                Err("alternative path node filter is too long".to_string())
            } else {
                Ok(value.to_string())
            }
        })
        .transpose()
}

fn requested_pairs(
    node_ids: &[String],
    requested_from: Option<&str>,
    requested_to: Option<&str>,
) -> Vec<(String, String)> {
    match (requested_from, requested_to) {
        (Some(from), Some(to)) if from != to => vec![(from.to_string(), to.to_string())],
        (Some(from), None) => node_ids
            .iter()
            .filter(|node_id| node_id.as_str() != from)
            .map(|to| (from.to_string(), to.clone()))
            .collect(),
        (None, Some(to)) => node_ids
            .iter()
            .filter(|node_id| node_id.as_str() != to)
            .map(|from| (from.clone(), to.to_string()))
            .collect(),
        (None, None) => node_ids
            .iter()
            .flat_map(|from| {
                node_ids
                    .iter()
                    .filter(move |to| *to != from)
                    .map(move |to| (from.clone(), to.clone()))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn enumerate_simple_paths(
    source: &str,
    target: &str,
    adjacency: &BTreeMap<String, Vec<ObservedEdge>>,
    max_hops: usize,
) -> Vec<EnumeratedPath> {
    let mut queue: VecDeque<(String, Vec<ObservedEdge>, HashSet<String>)> =
        VecDeque::from([(source.to_string(), Vec::new(), {
            let mut visited = HashSet::new();
            visited.insert(source.to_string());
            visited
        })]);
    let mut paths = Vec::new();

    while let Some((current, path_edges, visited)) = queue.pop_front() {
        if current == target && !path_edges.is_empty() {
            let mut path_nodes = Vec::with_capacity(path_edges.len() + 1);
            path_nodes.push(source.to_string());
            path_nodes.extend(path_edges.iter().map(|edge| edge.to.clone()));
            paths.push(EnumeratedPath {
                nodes: path_nodes,
                edges: path_edges,
            });
            continue;
        }
        if path_edges.len() >= max_hops {
            continue;
        }

        for edge in adjacency.get(&current).into_iter().flatten() {
            if visited.contains(&edge.to) {
                continue;
            }
            let mut next_visited = visited.clone();
            next_visited.insert(edge.to.clone());
            let mut next_edges = path_edges.clone();
            next_edges.push(edge.clone());
            queue.push_back((edge.to.clone(), next_edges, next_visited));
        }
    }
    paths
}

struct PathAssessment {
    status: &'static str,
    eligible: bool,
    reason: &'static str,
}

fn assess_path(path: &EnumeratedPath, nodes: &[(String, bool)]) -> PathAssessment {
    let node_health = path.nodes.iter().all(|node_id| {
        nodes
            .iter()
            .find(|(known_id, _)| known_id == node_id)
            .is_some_and(|(_, eligible)| *eligible)
    });
    let all_up = path.edges.iter().all(|edge| edge.edge_status == "up");
    let any_unreachable = path
        .edges
        .iter()
        .any(|edge| edge.edge_status == "down" || edge.edge_status == "degraded");
    let any_unknown = path.edges.iter().any(|edge| edge.edge_status == "unknown");

    if node_health && all_up {
        PathAssessment {
            status: "available",
            eligible: true,
            reason: "fresh_healthy_nodes_and_up_edges",
        }
    } else if any_unreachable {
        PathAssessment {
            status: "unavailable",
            eligible: false,
            reason: "path_contains_unreachable_or_degraded_edge",
        }
    } else if any_unknown {
        PathAssessment {
            status: "unknown",
            eligible: false,
            reason: "path_contains_unknown_edge_evidence",
        }
    } else if !node_health {
        PathAssessment {
            status: "degraded",
            eligible: false,
            reason: "path_contains_node_without_fresh_healthy_evidence",
        }
    } else {
        PathAssessment {
            status: "degraded",
            eligible: false,
            reason: "path_contains_stale_edge_evidence",
        }
    }
}

fn path_rank(assessment: &PathAssessment) -> u8 {
    match assessment.status {
        "available" => 0,
        "degraded" => 1,
        "unknown" => 2,
        "unavailable" => 3,
        _ => 4,
    }
}

fn path_key(path: &EnumeratedPath) -> String {
    path.nodes.join("->")
}

fn path_json(
    source: &str,
    target: &str,
    index: usize,
    path: &EnumeratedPath,
    assessment: &PathAssessment,
) -> Value {
    let edges = path
        .edges
        .iter()
        .map(|edge| {
            json!({
                "from": edge.from,
                "to": edge.to,
                "relation": edge.relation,
                "edge_status": edge.edge_status,
                "reachable": edge.reachable,
                "rtt_ms": edge.rtt_ms,
                "telemetry_freshness": edge.telemetry_freshness
            })
        })
        .collect::<Vec<_>>();

    json!({
        "path_id": format!("{}=>{}:{}", source, target, index),
        "from": source,
        "to": target,
        "nodes": path.nodes,
        "edges": edges,
        "hop_count": path.edges.len(),
        "path_status": assessment.status,
        "eligible": assessment.eligible,
        "reason": assessment.reason,
        "hypothetical": true
    })
}

fn edge_status(reachable: Option<bool>, freshness: &str) -> &'static str {
    match (reachable, freshness) {
        (Some(true), "fresh") => "up",
        (Some(true), _) => "stale",
        (Some(false), "fresh") => "down",
        (Some(false), _) => "degraded",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(edge_reachable: Value, edge_freshness: &str, failure_state: &str) -> Value {
        json!({
            "project_id": "monitor-f49c8",
            "dgraph_id": "dgraph:test",
            "revision": 5,
            "path_status": "all_expected_nodes_fresh",
            "nodes": [
                {
                    "node_id": "fog-01",
                    "observed": {"telemetry_freshness": "fresh"},
                    "failure_state": {"state": failure_state}
                },
                {
                    "node_id": "cloud-01",
                    "observed": {"telemetry_freshness": "fresh"},
                    "failure_state": {"state": "healthy"}
                }
            ],
            "measured_peer_edges": [
                {
                    "from": "fog-01",
                    "to": "cloud-01",
                    "relation": "peer_connectivity",
                    "reachable": edge_reachable,
                    "rtt_ms": 3.2,
                    "telemetry_freshness": edge_freshness
                }
            ]
        })
    }

    #[test]
    fn fresh_healthy_path_is_eligible_without_side_effects() {
        let plan = plan_alternative_paths(
            &observed(json!(true), "fresh", "healthy"),
            Some("fog-01"),
            Some("cloud-01"),
            None,
        )
        .unwrap();

        assert_eq!(plan["contract_kind"], "datum_alternative_path_what_if");
        assert_eq!(plan["summary"]["candidate_count"], 1);
        assert_eq!(plan["summary"]["eligible_count"], 1);
        assert_eq!(plan["candidates"][0]["path_status"], "available");
        assert_eq!(plan["candidates"][0]["eligible"], true);
        assert_eq!(plan["what_if"]["changes_applied"], false);
        assert_eq!(plan["side_effects_performed"], false);
    }

    #[test]
    fn stale_edge_is_visible_but_not_eligible() {
        let plan = plan_alternative_paths(
            &observed(json!(true), "stale", "healthy"),
            Some("fog-01"),
            Some("cloud-01"),
            None,
        )
        .unwrap();

        assert_eq!(plan["summary"]["ineligible_count"], 1);
        assert_eq!(plan["candidates"][0]["path_status"], "degraded");
        assert_eq!(plan["candidates"][0]["reason"], "path_contains_stale_edge_evidence");
    }

    #[test]
    fn unavailable_node_makes_path_ineligible() {
        let plan = plan_alternative_paths(
            &observed(json!(true), "fresh", "unavailable"),
            Some("fog-01"),
            Some("cloud-01"),
            None,
        )
        .unwrap();

        assert_eq!(plan["candidates"][0]["eligible"], false);
        assert_eq!(
            plan["candidates"][0]["reason"],
            "path_contains_node_without_fresh_healthy_evidence"
        );
    }

    #[test]
    fn missing_route_is_reported_as_rejected_pair() {
        let plan = plan_alternative_paths(
            &observed(json!(true), "fresh", "healthy"),
            Some("cloud-01"),
            Some("fog-01"),
            None,
        )
        .unwrap();

        assert_eq!(plan["summary"]["candidate_count"], 0);
        assert_eq!(plan["summary"]["rejected_pair_count"], 1);
        assert_eq!(
            plan["rejected_pairs"][0]["reason"],
            "no_observed_path_within_hop_limit"
        );
    }

    #[test]
    fn invalid_scope_node_is_rejected() {
        let error = plan_alternative_paths(
            &observed(json!(true), "fresh", "healthy"),
            Some("unknown"),
            Some("cloud-01"),
            None,
        )
        .unwrap_err();

        assert!(error.contains("not present in observed D-Graph"));
    }
}
