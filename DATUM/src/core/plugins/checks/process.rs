use serde_json::{json, Value as JsonValue};

pub fn run_process_alive(
    snapshot: &crate::core::model::Snapshot,
    service_attrs: &JsonValue,
    observed_at: &str,
) -> JsonValue {
    let bindings = service_attrs
        .get("bindings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for b in bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");

        if !bound || target_kind != "process" || target_id.is_empty() {
            continue;
        }

        if let Some(proc_ent) = snapshot.entities.iter().find(|e| e.id == target_id && e.kind == "process") {
            let instance_count = proc_ent
                .attrs
                .get("instance_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(1);

            if instance_count > 0 {
                return json!({
                    "kind": "process_alive",
                    "status": "available",
                    "target_id": target_id,
                    "instance_count": instance_count,
                    "checked_at": observed_at
                });
            }
        }
    }

    json!({
        "kind": "process_alive",
        "status": "unavailable",
        "error": "no_live_process_binding",
        "checked_at": observed_at
    })
}