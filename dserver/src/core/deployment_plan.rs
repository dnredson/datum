use std::collections::HashSet;

use serde_json::{json, Value};

pub fn empty_deployment_planning() -> Value {
    json!({
        "planner_version": "0.1.0",
        "items": [],
        "summary": {
            "total_items": 0,
            "high": 0,
            "medium": 0,
            "low": 0
        }
    })
}

pub fn derive_deployment_planning(action_plan: &Value) -> Value {
    let actions = action_plan
        .get("actions")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let items = derive_deployment_planning_items_from_actions(&actions);

    let mut high = 0_u64;
    let mut medium = 0_u64;
    let mut low = 0_u64;

    for item in &items {
        match item
            .get("priority")
            .and_then(|value| value.as_str())
            .unwrap_or("low")
        {
            "high" => high += 1,
            "medium" => medium += 1,
            _ => low += 1,
        }
    }

    json!({
        "planner_version": "0.1.0",
        "items": items,
        "summary": {
            "total_items": items.len(),
            "high": high,
            "medium": medium,
            "low": low
        }
    })
}

pub fn derive_deployment_planning_items_from_actions(actions: &[Value]) -> Vec<Value> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();

    for action in actions {
        let feedback_status = action
            .get("feedback")
            .and_then(|value| value.get("status"))
            .and_then(|value| value.as_str());
        let feedback_execution_status = action
            .get("feedback")
            .and_then(|value| value.get("execution_status"))
            .and_then(|value| value.as_str());
        let capability_decision = action
            .get("capability")
            .and_then(|value| value.get("decision"))
            .and_then(|value| value.as_str());

        if matches!(feedback_status, Some("matched")) {
            match feedback_execution_status {
                Some("missing_local_mapping") => {
                    push_unique_item(
                        &mut items,
                        &mut seen,
                        build_item(
                            action,
                            "add_local_mapping",
                            "high",
                            "client lacks trusted local mapping for this action",
                            "latest_action_results",
                        ),
                    );
                }
                Some("not_installed") => {
                    push_unique_item(
                        &mut items,
                        &mut seen,
                        build_item(
                            action,
                            "install_existing_artifact",
                            "high",
                            "target artifact/container/service is not installed locally",
                            "latest_action_results",
                        ),
                    );
                }
                Some("docker_unavailable") => {
                    push_unique_item(
                        &mut items,
                        &mut seen,
                        build_item(
                            action,
                            "enable_runtime",
                            "high",
                            "required container runtime is unavailable on target node",
                            "latest_action_results",
                        ),
                    );
                }
                Some("failed") => {
                    push_unique_item(
                        &mut items,
                        &mut seen,
                        build_item(
                            action,
                            "investigate_failure",
                            "medium",
                            "latest execution failed and retry policy is not implemented yet",
                            "latest_action_results",
                        ),
                    );
                }
                _ => {}
            }
        }

        match capability_decision {
            Some("unsupported") => {
                push_unique_item(
                    &mut items,
                    &mut seen,
                    build_item(
                        action,
                        "define_installation_strategy",
                        "medium",
                        "target node does not currently support required action/runtime",
                        "capability",
                    ),
                );
            }
            Some("unknown") => {
                push_unique_item(
                    &mut items,
                    &mut seen,
                    build_item(
                        action,
                        "unknown",
                        "low",
                        "capability state is unknown; publish node capabilities first",
                        "capability",
                    ),
                );
            }
            _ => {}
        }
    }

    items
}

fn push_unique_item(items: &mut Vec<Value>, seen: &mut HashSet<String>, item: Value) {
    let action_id = item
        .get("action_id")
        .and_then(|value| value.as_str())
        .unwrap_or("<none>");
    let required_step = item
        .get("required_step")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let reason = item
        .get("reason")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let key = format!("{action_id}|{required_step}|{reason}");

    if seen.insert(key) {
        items.push(item);
    }
}

fn build_item(
    action: &Value,
    required_step: &str,
    priority: &str,
    reason: &str,
    source: &str,
) -> Value {
    let action_id = action
        .get("action_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let module_id = action
        .get("module_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let target_node = action
        .get("target_node")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let planning_id = format!(
        "planning:action:{}:step:{}:node:{}",
        action_id.as_deref().unwrap_or("<none>"),
        required_step,
        target_node.as_deref().unwrap_or("<none>")
    );

    json!({
        "planning_id": planning_id,
        "action_id": action_id,
        "module_id": module_id,
        "module_kind": action.get("kind").cloned().unwrap_or(Value::Null),
        "runtime": action.get("runtime").cloned().unwrap_or(Value::Null),
        "artifact": action.get("artifact").cloned().unwrap_or(Value::Null),
        "target_node": target_node,
        "fallback_to": action.get("fallback_to").cloned().unwrap_or(Value::Null),
        "reason": reason,
        "required_step": required_step,
        "priority": priority,
        "source": source,
        "ai_context": action.get("ai_context").cloned().unwrap_or(Value::Null),
        "capability": action.get("capability").cloned().unwrap_or(Value::Null),
        "feedback": action.get("feedback").cloned().unwrap_or(Value::Null)
    })
}

pub fn filter_deployment_planning_for_node(deployment_planning: &Value, node_id: &str) -> Value {
    let items = deployment_planning
        .get("items")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    let planner_version = deployment_planning
        .get("planner_version")
        .cloned()
        .unwrap_or_else(|| json!("0.1.0"));

    let mut assigned_items = Vec::new();
    let mut high = 0_u64;
    let mut medium = 0_u64;
    let mut low = 0_u64;

    for mut item in items {
        let assignment_reason = if item
            .get("target_node")
            .and_then(|value| value.as_str())
            .map(|value| value == node_id)
            .unwrap_or(false)
        {
            Some("target_node")
        } else if item
            .get("fallback_to")
            .and_then(|value| value.as_str())
            .map(|value| value == node_id)
            .unwrap_or(false)
        {
            Some("fallback_node")
        } else {
            None
        };

        let Some(assignment_reason) = assignment_reason else {
            continue;
        };

        if let Some(item_obj) = item.as_object_mut() {
            item_obj.insert("assigned_node".to_string(), json!(node_id));
            item_obj.insert("assignment_reason".to_string(), json!(assignment_reason));
        }

        match item
            .get("priority")
            .and_then(|value| value.as_str())
            .unwrap_or("low")
        {
            "high" => high += 1,
            "medium" => medium += 1,
            _ => low += 1,
        }

        assigned_items.push(item);
    }

    json!({
        "planner_version": planner_version,
        "node_id": node_id,
        "items": assigned_items,
        "summary": {
            "total_items": assigned_items.len(),
            "assigned_items": assigned_items.len(),
            "high": high,
            "medium": medium,
            "low": low
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{
        derive_deployment_planning, derive_deployment_planning_items_from_actions,
        empty_deployment_planning, filter_deployment_planning_for_node,
    };
    use serde_json::{json, Value};

    fn base_action() -> Value {
        json!({
            "action_id": "gap:1",
            "module_id": "chirpstack-primary",
            "kind": "service",
            "runtime": "container",
            "artifact": "chirpstack/chirpstack:latest",
            "target_node": "fog-01",
            "ai_context": { "module_id": "chirpstack-primary" },
            "capability": { "decision": "supported", "node_id": "fog-01" },
            "feedback": { "status": "matched", "execution_status": "dry_run" }
        })
    }

    #[test]
    fn missing_local_mapping_creates_add_local_mapping_high_priority_item() {
        let mut action = base_action();
        action["feedback"]["execution_status"] = json!("missing_local_mapping");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0]["required_step"].as_str(),
            Some("add_local_mapping")
        );
        assert_eq!(items[0]["priority"].as_str(), Some("high"));
    }

    #[test]
    fn not_installed_creates_install_existing_artifact_high_priority_item() {
        let mut action = base_action();
        action["feedback"]["execution_status"] = json!("not_installed");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(
            items[0]["required_step"].as_str(),
            Some("install_existing_artifact")
        );
        assert_eq!(items[0]["priority"].as_str(), Some("high"));
    }

    #[test]
    fn docker_unavailable_creates_enable_runtime_high_priority_item() {
        let mut action = base_action();
        action["feedback"]["execution_status"] = json!("docker_unavailable");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(items[0]["required_step"].as_str(), Some("enable_runtime"));
        assert_eq!(items[0]["priority"].as_str(), Some("high"));
    }

    #[test]
    fn failed_creates_investigate_failure_medium_priority_item() {
        let mut action = base_action();
        action["feedback"]["execution_status"] = json!("failed");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(
            items[0]["required_step"].as_str(),
            Some("investigate_failure")
        );
        assert_eq!(items[0]["priority"].as_str(), Some("medium"));
    }

    #[test]
    fn unsupported_capability_creates_define_installation_strategy_medium_priority_item() {
        let mut action = base_action();
        action["capability"]["decision"] = json!("unsupported");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(
            items[0]["required_step"].as_str(),
            Some("define_installation_strategy")
        );
        assert_eq!(items[0]["priority"].as_str(), Some("medium"));
    }

    #[test]
    fn unknown_capability_creates_unknown_low_priority_item() {
        let mut action = base_action();
        action["capability"]["decision"] = json!("unknown");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(items[0]["required_step"].as_str(), Some("unknown"));
        assert_eq!(items[0]["priority"].as_str(), Some("low"));
    }

    #[test]
    fn dry_run_feedback_does_not_generate_planning_item() {
        let items = derive_deployment_planning_items_from_actions(&[base_action()]);
        assert!(items.is_empty());
    }

    #[test]
    fn already_running_feedback_does_not_generate_installation_item() {
        let mut action = base_action();
        action["feedback"]["execution_status"] = json!("already_running");
        let items = derive_deployment_planning_items_from_actions(&[action]);
        assert!(items.is_empty());
    }

    #[test]
    fn duplicate_action_reason_does_not_create_duplicate_items() {
        let mut action_a = base_action();
        action_a["feedback"]["execution_status"] = json!("failed");
        let mut action_b = base_action();
        action_b["feedback"]["execution_status"] = json!("failed");
        let items = derive_deployment_planning_items_from_actions(&[action_a, action_b]);

        assert_eq!(items.len(), 1);
    }

    #[test]
    fn summary_counts_total_high_medium_low_correctly() {
        let mut missing_local_mapping = base_action();
        missing_local_mapping["feedback"]["execution_status"] = json!("missing_local_mapping");

        let mut failed = base_action();
        failed["action_id"] = json!("gap:2");
        failed["feedback"]["execution_status"] = json!("failed");

        let mut unknown_capability = base_action();
        unknown_capability["action_id"] = json!("gap:3");
        unknown_capability["capability"]["decision"] = json!("unknown");
        unknown_capability["feedback"]["execution_status"] = json!("dry_run");

        let plan = derive_deployment_planning(&json!({
            "actions": [missing_local_mapping, failed, unknown_capability]
        }));

        assert_eq!(plan["summary"]["total_items"].as_u64(), Some(3));
        assert_eq!(plan["summary"]["high"].as_u64(), Some(1));
        assert_eq!(plan["summary"]["medium"].as_u64(), Some(1));
        assert_eq!(plan["summary"]["low"].as_u64(), Some(1));
    }

    #[test]
    fn ai_context_capability_and_feedback_are_preserved_in_items() {
        let mut action = base_action();
        action["feedback"]["execution_status"] = json!("missing_local_mapping");
        let items = derive_deployment_planning_items_from_actions(&[action]);

        assert_eq!(
            items[0]["ai_context"]["module_id"].as_str(),
            Some("chirpstack-primary")
        );
        assert_eq!(
            items[0]["capability"]["decision"].as_str(),
            Some("supported")
        );
        assert_eq!(
            items[0]["feedback"]["execution_status"].as_str(),
            Some("missing_local_mapping")
        );
    }

    #[test]
    fn item_with_target_node_matching_node_appears() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                {
                    "planning_id": "p1",
                    "target_node": "fog-01",
                    "priority": "high"
                }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(filtered["items"].as_array().map(|v| v.len()), Some(1));
    }

    #[test]
    fn item_with_fallback_to_matching_node_appears() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                {
                    "planning_id": "p1",
                    "fallback_to": "fog-01",
                    "priority": "medium"
                }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(filtered["items"].as_array().map(|v| v.len()), Some(1));
    }

    #[test]
    fn unrelated_item_is_filtered_out() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                {
                    "planning_id": "p1",
                    "target_node": "fog-02",
                    "priority": "high"
                }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(filtered["items"].as_array().map(|v| v.len()), Some(0));
    }

    #[test]
    fn assignment_reason_target_node_is_set_correctly() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                {
                    "planning_id": "p1",
                    "target_node": "fog-01",
                    "priority": "high"
                }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(
            filtered["items"][0]["assignment_reason"].as_str(),
            Some("target_node")
        );
    }

    #[test]
    fn assignment_reason_fallback_node_is_set_correctly() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                {
                    "planning_id": "p1",
                    "fallback_to": "fog-01",
                    "priority": "high"
                }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(
            filtered["items"][0]["assignment_reason"].as_str(),
            Some("fallback_node")
        );
    }

    #[test]
    fn empty_planning_returns_empty_items_and_zero_summary() {
        let filtered = filter_deployment_planning_for_node(&empty_deployment_planning(), "fog-01");
        assert_eq!(filtered["items"].as_array().map(|v| v.len()), Some(0));
        assert_eq!(filtered["summary"]["total_items"].as_u64(), Some(0));
        assert_eq!(filtered["summary"]["assigned_items"].as_u64(), Some(0));
    }

    #[test]
    fn node_filtered_summary_counts_high_medium_low_correctly() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                { "planning_id": "p1", "target_node": "fog-01", "priority": "high" },
                { "planning_id": "p2", "target_node": "fog-01", "priority": "medium" },
                { "planning_id": "p3", "fallback_to": "fog-01", "priority": "low" },
                { "planning_id": "p4", "target_node": "fog-02", "priority": "high" }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(filtered["summary"]["total_items"].as_u64(), Some(3));
        assert_eq!(filtered["summary"]["assigned_items"].as_u64(), Some(3));
        assert_eq!(filtered["summary"]["high"].as_u64(), Some(1));
        assert_eq!(filtered["summary"]["medium"].as_u64(), Some(1));
        assert_eq!(filtered["summary"]["low"].as_u64(), Some(1));
    }

    #[test]
    fn original_item_fields_are_preserved() {
        let planning = json!({
            "planner_version": "0.1.0",
            "items": [
                {
                    "planning_id": "p1",
                    "target_node": "fog-01",
                    "priority": "high",
                    "artifact": "chirpstack/chirpstack:latest",
                    "ai_context": { "module_id": "chirpstack-primary" }
                }
            ]
        });

        let filtered = filter_deployment_planning_for_node(&planning, "fog-01");
        assert_eq!(
            filtered["items"][0]["artifact"].as_str(),
            Some("chirpstack/chirpstack:latest")
        );
        assert_eq!(
            filtered["items"][0]["ai_context"]["module_id"].as_str(),
            Some("chirpstack-primary")
        );
    }
}
