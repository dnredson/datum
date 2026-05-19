use chrono::Utc;
use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::core::capabilities::{node_supports_action, CapabilityDecision, NodeCapabilities};

pub fn build_global_action_plan(
    project_id: &str,
    graph: &Value,
    analysis: &Value,
    dmap: Option<&Value>,
) -> Value {
    let entities = graph
        .get("entities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let impacted = analysis
        .get("impacted_entities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let modules = dmap
        .and_then(|v| v.get("modules"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut actions = Vec::new();

    for module in modules {
        let module_id = module
            .get("module_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let kind = module
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let runtime = module
            .get("runtime")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let artifact = module
            .get("artifact")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let desired_node = module
            .get("desired_node")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let desired_state = module
            .get("desired_state")
            .and_then(|v| v.as_str())
            .unwrap_or("running");

        let fallback_nodes = module
            .get("fallback_nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let migration_policy = module.get("migration_policy").cloned().unwrap_or(json!({}));

        let reconfiguration = module.get("reconfiguration").cloned().unwrap_or(json!({}));

        let expected_gid = format!(
            "project:{}:node:{}:entity:service:{}",
            project_id, desired_node, module_id
        );

        let found = entities.iter().find(|ent| {
            ent.get("id")
                .and_then(|v| v.as_str())
                .map(|id| id == expected_gid)
                .unwrap_or(false)
        });

        let observed_state = found
            .and_then(|ent| ent.get("data"))
            .and_then(|v| v.get("attrs"))
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("missing");

        let impacted_match = impacted.iter().any(|x| {
            x.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s == expected_gid)
                .unwrap_or(false)
        });

        let mut reasons = Vec::new();

        if found.is_none() {
            reasons.push("module_missing_from_graph".to_string());
        }

        if observed_state != desired_state {
            reasons.push(format!(
                "desired_state_mismatch:{}->{}",
                observed_state, desired_state
            ));
        }

        if impacted_match {
            reasons.push("module_marked_as_impacted".to_string());
        }

        if reasons.is_empty() {
            continue;
        }

        let fallback_to = fallback_nodes
            .first()
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let action_kind = if impacted_match && !fallback_to.is_empty() {
            match runtime {
                "container" => "migrate_container",
                "wasm" => "migrate_wasm",
                _ => "migrate_module",
            }
        } else {
            match runtime {
                "container" => "ensure_container_running",
                "wasm" => "ensure_wasm_running",
                _ => "ensure_module_running",
            }
        };

        actions.push(json!({
            "action_id": format!("gap:{}:{}:{}", project_id, desired_node, module_id),
            "module_id": module_id,
            "kind": kind,
            "runtime": runtime,
            "artifact": artifact,
            "target_node": desired_node,
            "fallback_to": fallback_to,
            "desired_state": desired_state,
            "observed_state": observed_state,
            "action_kind": action_kind,
            "priority": if impacted_match { "high" } else { "medium" },
            "reasons": reasons,
            "migration_policy": migration_policy,
            "reconfiguration": reconfiguration,
            "ai_context": {
                "kind": "module_gap",
                "project_id": project_id,
                "entity_id": expected_gid,
                "module_id": module_id,
                "module_kind": kind,
                "target_node": desired_node,
                "runtime": runtime,
                "artifact": artifact,
                "observed_state": observed_state,
                "desired_state": desired_state,
                "fallback_nodes": fallback_nodes
            }
        }));
    }

    json!({
        "project_id": project_id,
        "generated_at_utc": Utc::now().to_rfc3339(),
        "planner_version": "0.2.0",
        "ai_ready": true,
        "summary": {
            "actions": actions.len()
        },
        "actions": actions
    })
}

pub fn annotate_action_plan_with_capabilities(
    mut plan: Value,
    capabilities_by_node: &BTreeMap<String, NodeCapabilities>,
) -> Value {
    let Some(actions) = plan.get_mut("actions").and_then(|v| v.as_array_mut()) else {
        return plan;
    };

    let mut supported = 0_u64;
    let mut unsupported = 0_u64;
    let mut unknown = 0_u64;

    for action in actions.iter_mut() {
        let capability = build_action_capability_metadata(action, capabilities_by_node);
        match capability
            .get("decision")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown")
        {
            "supported" => supported += 1,
            "unsupported" => unsupported += 1,
            _ => unknown += 1,
        }

        if let Some(action_obj) = action.as_object_mut() {
            action_obj.insert("capability".to_string(), capability);
        }
    }

    if let Some(plan_obj) = plan.as_object_mut() {
        plan_obj.insert(
            "capability_summary".to_string(),
            json!({
                "supported": supported,
                "unsupported": unsupported,
                "unknown": unknown
            }),
        );
    }

    plan
}

fn build_action_capability_metadata(
    action: &Value,
    capabilities_by_node: &BTreeMap<String, NodeCapabilities>,
) -> Value {
    let node_id = action
        .get("target_node")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let action_kind = action
        .get("action_kind")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let runtime = action
        .get("runtime")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let Some(node_id) = node_id else {
        return json!({
            "decision": "unknown",
            "reason": "action has no target node",
            "node_id": Value::Null,
            "source": "missing"
        });
    };

    let Some(action_kind) = action_kind else {
        return json!({
            "decision": "unknown",
            "reason": "action has no action_kind",
            "node_id": node_id,
            "source": "missing"
        });
    };

    let Some(runtime) = runtime else {
        return json!({
            "decision": "unknown",
            "reason": "action has no runtime",
            "node_id": node_id,
            "source": "missing"
        });
    };

    let Some(capabilities) = capabilities_by_node.get(node_id) else {
        return json!({
            "decision": "unknown",
            "reason": "no capabilities registered for target node",
            "node_id": node_id,
            "source": "missing"
        });
    };

    match node_supports_action(capabilities, action_kind, runtime) {
        CapabilityDecision::Supported => json!({
            "decision": "supported",
            "reason": "target node capabilities explicitly support this action",
            "node_id": node_id,
            "source": "registered"
        }),
        CapabilityDecision::Unsupported { reason } => json!({
            "decision": "unsupported",
            "reason": reason,
            "node_id": node_id,
            "source": "registered"
        }),
        CapabilityDecision::Unknown { reason } => json!({
            "decision": "unknown",
            "reason": reason,
            "node_id": node_id,
            "source": "registered"
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::annotate_action_plan_with_capabilities;
    use crate::core::capabilities::{
        ContainerCapabilities, NodeCapabilities, RuntimeCapabilities, WasmCapabilities,
    };
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    fn container_caps(
        available: bool,
        allow_execution: bool,
        supports_existing_start: bool,
    ) -> NodeCapabilities {
        NodeCapabilities {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            stage: Some("fog".to_string()),
            updated_at_utc: "2026-05-19T13:00:00Z".to_string(),
            source: Some("manual".to_string()),
            runtimes: RuntimeCapabilities {
                container: Some(ContainerCapabilities {
                    available,
                    engine: Some("docker".to_string()),
                    allow_execution: Some(allow_execution),
                    supports_existing_start: Some(supports_existing_start),
                    supports_restart: Some(false),
                    supports_create: Some(false),
                    supports_pull: Some(false),
                    ..ContainerCapabilities::default()
                }),
                wasm: Some(WasmCapabilities {
                    available: false,
                    runtime: None,
                    allow_execution: Some(false),
                    supports_wasi: Some(false),
                    supports_http: Some(false),
                    supports_mqtt: Some(false),
                    ..WasmCapabilities::default()
                }),
                ..RuntimeCapabilities::default()
            },
            ..NodeCapabilities::default()
        }
    }

    fn action_plan_with(actions: Vec<Value>) -> Value {
        json!({
            "project_id": "smart-city-campinas",
            "generated_at_utc": "2026-05-19T13:00:00Z",
            "planner_version": "0.2.0",
            "ai_ready": true,
            "summary": {
                "actions": actions.len()
            },
            "actions": actions
        })
    }

    #[test]
    fn annotates_supported_container_action() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("fog-01".to_string(), container_caps(true, true, true));

        let plan = annotate_action_plan_with_capabilities(
            action_plan_with(vec![json!({
                "action_id": "gap:1",
                "action_kind": "ensure_container_running",
                "runtime": "container",
                "target_node": "fog-01"
            })]),
            &capabilities,
        );

        assert_eq!(
            plan["actions"][0]["capability"]["decision"].as_str(),
            Some("supported")
        );
    }

    #[test]
    fn annotates_unsupported_when_container_unavailable() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("fog-01".to_string(), container_caps(false, false, false));

        let plan = annotate_action_plan_with_capabilities(
            action_plan_with(vec![json!({
                "action_id": "gap:1",
                "action_kind": "ensure_container_running",
                "runtime": "container",
                "target_node": "fog-01"
            })]),
            &capabilities,
        );

        assert_eq!(
            plan["actions"][0]["capability"]["decision"].as_str(),
            Some("unsupported")
        );
    }

    #[test]
    fn annotates_unknown_when_no_capabilities_exist_for_node() {
        let plan = annotate_action_plan_with_capabilities(
            action_plan_with(vec![json!({
                "action_id": "gap:1",
                "action_kind": "ensure_container_running",
                "runtime": "container",
                "target_node": "fog-02"
            })]),
            &BTreeMap::new(),
        );

        assert_eq!(
            plan["actions"][0]["capability"]["decision"].as_str(),
            Some("unknown")
        );
        assert_eq!(
            plan["actions"][0]["capability"]["reason"].as_str(),
            Some("no capabilities registered for target node")
        );
    }

    #[test]
    fn annotates_wasm_action_as_unsupported_when_wasm_unavailable() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("fog-01".to_string(), container_caps(true, true, true));

        let plan = annotate_action_plan_with_capabilities(
            action_plan_with(vec![json!({
                "action_id": "gap:2",
                "action_kind": "ensure_wasm_running",
                "runtime": "wasm",
                "target_node": "fog-01"
            })]),
            &capabilities,
        );

        assert_eq!(
            plan["actions"][0]["capability"]["decision"].as_str(),
            Some("unsupported")
        );
    }

    #[test]
    fn annotates_migration_action_as_unknown() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("fog-01".to_string(), container_caps(true, true, true));

        let plan = annotate_action_plan_with_capabilities(
            action_plan_with(vec![json!({
                "action_id": "gap:3",
                "action_kind": "migrate_container",
                "runtime": "container",
                "target_node": "fog-01"
            })]),
            &capabilities,
        );

        assert_eq!(
            plan["actions"][0]["capability"]["decision"].as_str(),
            Some("unknown")
        );
        assert!(plan["actions"][0]["capability"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("migration capability planning is not implemented yet"));
    }

    #[test]
    fn capability_summary_counts_supported_unsupported_and_unknown() {
        let mut capabilities = BTreeMap::new();
        capabilities.insert("fog-01".to_string(), container_caps(true, true, true));
        capabilities.insert("fog-02".to_string(), container_caps(false, false, false));

        let plan = annotate_action_plan_with_capabilities(
            action_plan_with(vec![
                json!({
                    "action_id": "gap:1",
                    "action_kind": "ensure_container_running",
                    "runtime": "container",
                    "target_node": "fog-01"
                }),
                json!({
                    "action_id": "gap:2",
                    "action_kind": "ensure_container_running",
                    "runtime": "container",
                    "target_node": "fog-02"
                }),
                json!({
                    "action_id": "gap:3",
                    "action_kind": "ensure_container_running",
                    "runtime": "container",
                    "target_node": "fog-03"
                }),
            ]),
            &capabilities,
        );

        assert_eq!(plan["capability_summary"]["supported"].as_u64(), Some(1));
        assert_eq!(plan["capability_summary"]["unsupported"].as_u64(), Some(1));
        assert_eq!(plan["capability_summary"]["unknown"].as_u64(), Some(1));
    }
}
