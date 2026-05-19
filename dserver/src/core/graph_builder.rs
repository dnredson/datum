use chrono::Utc;
use serde_json::{json, Value};

pub fn build_latest_graph(project_id: &str, docs: &[Value]) -> Value {
    let mut project_nodes = Vec::new();
    let mut entities = Vec::new();
    let mut edges = Vec::new();
    let mut events = Vec::new();

    for doc in docs {
        let fields = doc.get("fields").cloned().unwrap_or_default();

        let node_id = get_string(&fields, "node_id").unwrap_or("unknown-node".into());
        let node_gid = format!("project:{}:node:{}", project_id, node_id);

        project_nodes.push(json!({
            "id": node_gid,
            "kind": "node",
            "project_id": project_id,
            "node_id": node_id
        }));

        let payload = get_string(&fields, "payload")
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or_default();

        let snapshot = payload.get("snapshot").cloned().unwrap_or_default();

        let local_entities = snapshot
            .get("entities")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let local_edges = snapshot
            .get("edges")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let local_events = snapshot
            .get("events")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for ent in local_entities {
            let local_id = ent.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");

            let ent_gid = format!(
                "project:{}:node:{}:entity:{}",
                project_id, node_id, local_id
            );

            entities.push(json!({
                "id": ent_gid,
                "kind": ent.get("kind").cloned().unwrap_or(json!("unknown")),
                "project_id": project_id,
                "node_id": node_id,
                "local_id": local_id,
                "data": ent
            }));

            edges.push(json!({
                "from": node_gid,
                "to": ent_gid,
                "rel": "contains"
            }));
        }

        for e in local_edges {
            let from = e
                .get("from")
                .or_else(|| e.get("source"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let to = e
                .get("to")
                .or_else(|| e.get("target"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let rel = e
                .get("rel")
                .or_else(|| e.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("related_to");

            let from_gid = format!("project:{}:node:{}:entity:{}", project_id, node_id, from);

            let to_gid = format!("project:{}:node:{}:entity:{}", project_id, node_id, to);

            edges.push(json!({
                "from": from_gid,
                "to": to_gid,
                "rel": rel,
                "data": e
            }));
        }

        for ev in local_events {
            let local_id = ev
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown-event");

            let target = ev.get("target").and_then(|v| v.as_str()).unwrap_or("");

            let target_gid = if target.is_empty() {
                Value::Null
            } else {
                json!(format!(
                    "project:{}:node:{}:entity:{}",
                    project_id, node_id, target
                ))
            };

            events.push(json!({
                "id": format!("project:{}:node:{}:event:{}", project_id, node_id, local_id),
                "project_id": project_id,
                "node_id": node_id,
                "local_id": local_id,
                "kind": ev.get("kind").cloned().unwrap_or(json!("unknown")),
                "severity": ev.get("severity").cloned().unwrap_or(json!("unknown")),
                "status": ev.get("status").cloned().unwrap_or(json!("unknown")),
                "target": target_gid,
                "data": ev
            }));
        }
    }

    json!({
        "project_id": project_id,
        "generated_at_utc": Utc::now().to_rfc3339(),
        "stats": {
            "nodes": project_nodes.len(),
            "entities": entities.len(),
            "edges": edges.len(),
            "events": events.len()
        },
        "project_nodes": project_nodes,
        "entities": entities,
        "edges": edges,
        "events": events
    })
}

fn get_string(fields: &Value, key: &str) -> Option<String> {
    fields
        .get(key)?
        .get("stringValue")?
        .as_str()
        .map(|s| s.to_string())
}
