use std::collections::HashSet;

use serde_json::{json, Value};

use crate::core::deployment_plan::filter_deployment_planning_for_node;

pub fn empty_installation_plan() -> Value {
    json!({
        "installer_version": "0.1.0",
        "source": "deployment_planning",
        "items": [],
        "summary": {
            "total_items": 0,
            "operator_required": 0,
            "auto_executable": 0,
            "high": 0,
            "medium": 0,
            "low": 0
        }
    })
}

pub fn derive_installation_plan_from_deployment_planning(deployment_planning: &Value) -> Value {
    let planning_items = deployment_planning
        .get("items")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = Vec::new();
    let mut seen_planning_ids = HashSet::new();
    let mut operator_required = 0_u64;
    let auto_executable = 0_u64;
    let mut high = 0_u64;
    let mut medium = 0_u64;
    let mut low = 0_u64;

    for planning_item in planning_items {
        let planning_id = planning_item
            .get("planning_id")
            .and_then(|value| value.as_str())
            .map(ToString::to_string);

        if let Some(planning_id) = planning_id.as_ref() {
            if !seen_planning_ids.insert(planning_id.clone()) {
                continue;
            }
        }

        let required_step = planning_item
            .get("required_step")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let installation_action = map_installation_action(required_step);
        let priority = planning_item
            .get("priority")
            .and_then(|value| value.as_str())
            .unwrap_or("low");

        match priority {
            "high" => high += 1,
            "medium" => medium += 1,
            _ => low += 1,
        }

        operator_required += 1;

        let installation_id = format!(
            "installation:{}:{}",
            planning_id.as_deref().unwrap_or("<none>"),
            installation_action
        );

        items.push(json!({
            "installation_id": installation_id,
            "planning_id": planning_id,
            "action_id": planning_item.get("action_id").cloned().unwrap_or(Value::Null),
            "node_id": planning_item
                .get("assigned_node")
                .cloned()
                .or_else(|| planning_item.get("target_node").cloned())
                .unwrap_or(Value::Null),
            "module_id": planning_item.get("module_id").cloned().unwrap_or(Value::Null),
            "runtime": planning_item.get("runtime").cloned().unwrap_or(Value::Null),
            "artifact": planning_item.get("artifact").cloned().unwrap_or(Value::Null),
            "required_step": required_step,
            "installation_action": installation_action,
            "status": "planned",
            "priority": priority,
            "reason": planning_item.get("reason").cloned().unwrap_or(Value::Null),
            "requires_operator": true,
            "safe_to_auto_execute": false,
            "source_planning_item": planning_item
        }));
    }

    json!({
        "installer_version": "0.1.0",
        "source": "deployment_planning",
        "items": items,
        "summary": {
            "total_items": items.len(),
            "operator_required": operator_required,
            "auto_executable": auto_executable,
            "high": high,
            "medium": medium,
            "low": low
        }
    })
}

pub fn derive_installation_plan_for_node(deployment_planning: &Value, node_id: &str) -> Value {
    let scoped = filter_deployment_planning_for_node(deployment_planning, node_id);
    derive_installation_plan_from_deployment_planning(&scoped)
}

fn map_installation_action(required_step: &str) -> &'static str {
    match required_step {
        "add_local_mapping" => "prepare_local_mapping",
        "install_existing_artifact" => "prepare_existing_artifact_installation",
        "enable_runtime" => "prepare_runtime_enablement",
        "investigate_failure" => "investigate_previous_failure",
        "define_installation_strategy" => "define_installation_strategy",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        derive_installation_plan_for_node, derive_installation_plan_from_deployment_planning,
        empty_installation_plan,
    };
    use serde_json::json;

    fn base_planning_item() -> serde_json::Value {
        json!({
            "planning_id": "planning:1",
            "action_id": "gap:1",
            "module_id": "chirpstack-primary",
            "runtime": "container",
            "artifact": "chirpstack/chirpstack:latest",
            "target_node": "fog-01",
            "priority": "high",
            "reason": "client lacks trusted local mapping for this action",
            "required_step": "add_local_mapping",
            "source": "latest_action_results",
            "ai_context": { "module_id": "chirpstack-primary" },
            "capability": { "decision": "supported" },
            "feedback": { "status": "matched", "execution_status": "missing_local_mapping" }
        })
    }

    #[test]
    fn add_local_mapping_maps_to_prepare_local_mapping() {
        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": [base_planning_item()]
        }));

        assert_eq!(
            plan["items"][0]["installation_action"].as_str(),
            Some("prepare_local_mapping")
        );
    }

    #[test]
    fn install_existing_artifact_maps_correctly() {
        let mut item = base_planning_item();
        item["required_step"] = json!("install_existing_artifact");

        let plan = derive_installation_plan_from_deployment_planning(&json!({ "items": [item] }));

        assert_eq!(
            plan["items"][0]["installation_action"].as_str(),
            Some("prepare_existing_artifact_installation")
        );
    }

    #[test]
    fn enable_runtime_maps_correctly() {
        let mut item = base_planning_item();
        item["required_step"] = json!("enable_runtime");

        let plan = derive_installation_plan_from_deployment_planning(&json!({ "items": [item] }));

        assert_eq!(
            plan["items"][0]["installation_action"].as_str(),
            Some("prepare_runtime_enablement")
        );
    }

    #[test]
    fn investigate_failure_maps_correctly() {
        let mut item = base_planning_item();
        item["required_step"] = json!("investigate_failure");
        item["priority"] = json!("medium");

        let plan = derive_installation_plan_from_deployment_planning(&json!({ "items": [item] }));

        assert_eq!(
            plan["items"][0]["installation_action"].as_str(),
            Some("investigate_previous_failure")
        );
    }

    #[test]
    fn define_installation_strategy_maps_correctly() {
        let mut item = base_planning_item();
        item["required_step"] = json!("define_installation_strategy");
        item["priority"] = json!("medium");

        let plan = derive_installation_plan_from_deployment_planning(&json!({ "items": [item] }));

        assert_eq!(
            plan["items"][0]["installation_action"].as_str(),
            Some("define_installation_strategy")
        );
    }

    #[test]
    fn unknown_maps_to_unknown() {
        let mut item = base_planning_item();
        item["required_step"] = json!("unknown");
        item["priority"] = json!("low");

        let plan = derive_installation_plan_from_deployment_planning(&json!({ "items": [item] }));

        assert_eq!(
            plan["items"][0]["installation_action"].as_str(),
            Some("unknown")
        );
    }

    #[test]
    fn all_items_have_planned_status() {
        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": [base_planning_item()]
        }));

        assert_eq!(plan["items"][0]["status"].as_str(), Some("planned"));
    }

    #[test]
    fn all_items_are_non_executable_and_operator_required() {
        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": [base_planning_item()]
        }));

        assert_eq!(
            plan["items"][0]["safe_to_auto_execute"].as_bool(),
            Some(false)
        );
        assert_eq!(plan["items"][0]["requires_operator"].as_bool(), Some(true));
    }

    #[test]
    fn summary_counts_are_correct() {
        let mut medium_item = base_planning_item();
        medium_item["planning_id"] = json!("planning:2");
        medium_item["required_step"] = json!("investigate_failure");
        medium_item["priority"] = json!("medium");

        let mut low_item = base_planning_item();
        low_item["planning_id"] = json!("planning:3");
        low_item["required_step"] = json!("unknown");
        low_item["priority"] = json!("low");

        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": [base_planning_item(), medium_item, low_item]
        }));

        assert_eq!(plan["summary"]["total_items"].as_u64(), Some(3));
        assert_eq!(plan["summary"]["operator_required"].as_u64(), Some(3));
        assert_eq!(plan["summary"]["auto_executable"].as_u64(), Some(0));
        assert_eq!(plan["summary"]["high"].as_u64(), Some(1));
        assert_eq!(plan["summary"]["medium"].as_u64(), Some(1));
        assert_eq!(plan["summary"]["low"].as_u64(), Some(1));
    }

    #[test]
    fn empty_deployment_planning_returns_empty_installation_plan() {
        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": []
        }));

        assert_eq!(plan, empty_installation_plan());
    }

    #[test]
    fn source_planning_item_is_preserved() {
        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": [base_planning_item()]
        }));

        assert_eq!(
            plan["items"][0]["source_planning_item"]["module_id"].as_str(),
            Some("chirpstack-primary")
        );
        assert_eq!(
            plan["items"][0]["source_planning_item"]["feedback"]["execution_status"].as_str(),
            Some("missing_local_mapping")
        );
    }

    #[test]
    fn duplicate_planning_id_does_not_create_duplicate_installation_items() {
        let item = base_planning_item();
        let plan = derive_installation_plan_from_deployment_planning(&json!({
            "items": [item.clone(), item]
        }));

        assert_eq!(plan["items"].as_array().map(|items| items.len()), Some(1));
    }

    #[test]
    fn derive_for_node_reuses_node_filtering() {
        let mut other_node = base_planning_item();
        other_node["planning_id"] = json!("planning:other");
        other_node["target_node"] = json!("fog-02");

        let plan = derive_installation_plan_for_node(
            &json!({
                "items": [base_planning_item(), other_node]
            }),
            "fog-01",
        );

        assert_eq!(plan["items"].as_array().map(|items| items.len()), Some(1));
        assert_eq!(plan["items"][0]["node_id"].as_str(), Some("fog-01"));
    }
}
