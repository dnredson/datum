use serde_json::{json, Value};

pub fn apply_topology(mut graph: Value, topology: Option<&Value>) -> Value {
    let Some(topology) = topology else {
        return graph;
    };

    let Some(graph_obj) = graph.as_object_mut() else {
        return graph;
    };

    // garantir estruturas
    if !graph_obj.contains_key("edges") {
        graph_obj.insert("edges".to_string(), json!([]));
    }

    if !graph_obj.contains_key("external_nodes") {
        graph_obj.insert("external_nodes".to_string(), json!([]));
    }

    let links = topology
        .get("links")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut cross_edges = Vec::new();
    let mut external_nodes = Vec::new();

    for link in links {
        let from = link.get("from").and_then(|v| v.as_str()).unwrap_or("");
        let to = link.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let rel = link
            .get("rel")
            .and_then(|v| v.as_str())
            .unwrap_or("related_to");
        let kind = link
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("cross_node");

        if from.is_empty() || to.is_empty() {
            continue;
        }

        cross_edges.push(json!({
            "from": from,
            "to": to,
            "rel": rel,
            "kind": kind
        }));

        if to.contains(":external:") {
            external_nodes.push(json!({
                "id": to,
                "kind": "external"
            }));
        }
    }

    if let Some(edges_arr) = graph_obj.get_mut("edges").and_then(|v| v.as_array_mut()) {
        edges_arr.extend(cross_edges);
    }

    if let Some(ext_arr) = graph_obj
        .get_mut("external_nodes")
        .and_then(|v| v.as_array_mut())
    {
        ext_arr.extend(external_nodes);
    }

    // atualizar stats
    let new_edges = graph_obj
        .get("edges")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);

    if let Some(stats) = graph_obj.get_mut("stats").and_then(|v| v.as_object_mut()) {
        stats.insert("edges".to_string(), json!(new_edges));
    }

    graph
}
