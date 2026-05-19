use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn analyze_graph(graph: &Value) -> Value {
    let entities = graph
        .get("entities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let edges = graph
        .get("edges")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let events = graph
        .get("events")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

    for e in &edges {
        let from = e
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let to = e
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !from.is_empty() && !to.is_empty() {
            adjacency.entry(from).or_default().push(to);
        }
    }

    let mut degraded_roots = Vec::new();

    for ent in &entities {
        let id = ent.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let data = ent.get("data").cloned().unwrap_or_default();

        let health = data
            .get("health")
            .or_else(|| data.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if matches!(health, "down" | "failed" | "degraded" | "unhealthy") {
            degraded_roots.push(id.to_string());
        }
    }

    for ev in &events {
        let status = ev.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let severity = ev.get("severity").and_then(|v| v.as_str()).unwrap_or("");
        let target = ev.get("target").and_then(|v| v.as_str()).unwrap_or("");

        if !target.is_empty() && (status == "open" || severity == "critical" || severity == "high")
        {
            degraded_roots.push(target.to_string());
        }
    }

    degraded_roots.sort();
    degraded_roots.dedup();

    let mut impacted: HashSet<String> = HashSet::new();
    let mut q: VecDeque<String> = degraded_roots.iter().cloned().collect();

    while let Some(cur) = q.pop_front() {
        if !impacted.insert(cur.clone()) {
            continue;
        }

        if let Some(nexts) = adjacency.get(&cur) {
            for n in nexts {
                if !impacted.contains(n) {
                    q.push_back(n.clone());
                }
            }
        }
    }

    let impacted_vec: Vec<Value> = impacted.iter().map(|id| json!({ "id": id })).collect();

    json!({
        "project_id": graph.get("project_id").cloned().unwrap_or(json!(null)),
        "generated_at_utc": chrono::Utc::now().to_rfc3339(),
        "summary": {
            "degraded_roots": degraded_roots.len(),
            "impacted_entities": impacted.len()
        },
        "degraded_roots": degraded_roots,
        "impacted_entities": impacted_vec
    })
}
