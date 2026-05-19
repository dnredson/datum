use chrono::Utc;
use serde_json::{json, Value};

pub fn build_global_events(analysis: &Value) -> Value {
    let project_id = analysis.get("project_id").cloned().unwrap_or(json!(null));
    let degraded_roots = analysis
        .get("degraded_roots")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let impacted_entities = analysis
        .get("impacted_entities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut events = Vec::new();

    for root in &degraded_roots {
        let root_id = root.as_str().unwrap_or("unknown");

        events.push(json!({
            "id": format!("global-event:root-cause:{}", sanitize_id(root_id)),
            "kind": "root_cause_candidate",
            "severity": "high",
            "status": "open",
            "target": root_id,
            "generated_at_utc": Utc::now().to_rfc3339()
        }));
    }

    if !impacted_entities.is_empty() {
        events.push(json!({
            "id": format!("global-event:impact-summary:{}", Utc::now().timestamp_millis()),
            "kind": "impact_summary",
            "severity": "medium",
            "status": "open",
            "generated_at_utc": Utc::now().to_rfc3339(),
            "summary": {
                "impacted_entities": impacted_entities.len()
            }
        }));
    }

    json!({
        "project_id": project_id,
        "generated_at_utc": Utc::now().to_rfc3339(),
        "events": events,
        "stats": {
            "events": events.len()
        }
    })
}

fn sanitize_id(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
