use chrono::Utc;
use serde_json::{json, Value};

pub fn apply_dispatch_policy(action_plan: &Value) -> Value {
    let actions = action_plan
        .get("actions")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();

    let mut dispatchable_actions = Vec::new();
    let mut blocked_actions = Vec::new();
    let mut deferred_actions = Vec::new();

    for action in actions {
        let normalized = ensure_capability_metadata(action);
        let decision = normalized
            .get("capability")
            .and_then(|value| value.get("decision"))
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");

        match decision {
            "supported" => dispatchable_actions.push(normalized),
            "unsupported" => blocked_actions.push(normalized),
            _ => deferred_actions.push(normalized),
        }
    }

    json!({
        "dispatch_version": "0.2.0",
        "project_id": action_plan.get("project_id").cloned().unwrap_or(Value::Null),
        "generated_at_utc": Utc::now().to_rfc3339(),
        "source_action_plan_generated_at_utc": action_plan
            .get("generated_at_utc")
            .cloned()
            .unwrap_or(Value::Null),
        "planner_version": action_plan.get("planner_version").cloned().unwrap_or(Value::Null),
        "ai_ready": action_plan.get("ai_ready").cloned().unwrap_or(Value::Bool(true)),
        "capability_summary": action_plan
            .get("capability_summary")
            .cloned()
            .unwrap_or_else(|| json!({
                "supported": dispatchable_actions.len(),
                "unsupported": blocked_actions.len(),
                "unknown": deferred_actions.len()
            })),
        "actions": dispatchable_actions,
        "blocked_actions": blocked_actions,
        "deferred_actions": deferred_actions,
        "dispatch_policy": {
            "policy_version": "0.1.0",
            "mode": "capability_gate",
            "supported_actions_dispatched": true,
            "unsupported_actions_blocked": true,
            "unknown_actions_deferred": true
        },
        "summary": {
            "total_actions": action_plan
                .get("actions")
                .and_then(|value| value.as_array())
                .map(|value| value.len())
                .unwrap_or(0),
            "dispatchable_actions": dispatchable_actions.len(),
            "blocked_actions": blocked_actions.len(),
            "deferred_actions": deferred_actions.len()
        }
    })
}

fn ensure_capability_metadata(mut action: Value) -> Value {
    let has_capability = action
        .get("capability")
        .and_then(|value| value.as_object())
        .is_some();

    if has_capability {
        return action;
    }

    let node_id = action.get("target_node").cloned().unwrap_or(Value::Null);

    if let Some(obj) = action.as_object_mut() {
        obj.insert(
            "capability".to_string(),
            json!({
                "decision": "unknown",
                "reason": "capability metadata missing",
                "node_id": node_id,
                "source": "missing"
            }),
        );
    }

    action
}

pub fn filter_dispatch_for_node(dispatch: &Value, project_id: &str, node_id: &str) -> Value {
    let actions = filter_actions_for_node(dispatch.get("actions"), node_id);
    let blocked_actions = filter_actions_for_node(dispatch.get("blocked_actions"), node_id);
    let deferred_actions = filter_actions_for_node(dispatch.get("deferred_actions"), node_id);

    json!({
        "project_id": project_id,
        "node_id": node_id,
        "dispatch_version": dispatch.get("dispatch_version").cloned().unwrap_or(Value::Null),
        "generated_at_utc": dispatch.get("generated_at_utc").cloned().unwrap_or(Value::Null),
        "source_action_plan_generated_at_utc": dispatch
            .get("source_action_plan_generated_at_utc")
            .cloned()
            .unwrap_or(Value::Null),
        "dispatch_policy": dispatch.get("dispatch_policy").cloned().unwrap_or(Value::Null),
        "summary": {
            "total_actions": dispatch
                .get("summary")
                .and_then(|summary| summary.get("total_actions"))
                .cloned()
                .unwrap_or_else(|| json!(actions.len() + blocked_actions.len() + deferred_actions.len())),
            "assigned_actions": actions.len(),
            "assigned_blocked_actions": blocked_actions.len(),
            "assigned_deferred_actions": deferred_actions.len()
        },
        "actions": actions,
        "blocked_actions": blocked_actions,
        "deferred_actions": deferred_actions
    })
}

fn filter_actions_for_node(actions: Option<&Value>, node_id: &str) -> Vec<Value> {
    actions
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|action| action_matches_node(action, node_id))
        .collect()
}

fn action_matches_node(action: &Value, node_id: &str) -> bool {
    let target_match = action
        .get("target_node")
        .and_then(|value| value.as_str())
        .map(|target| target == node_id)
        .unwrap_or(false);

    let fallback_match = action
        .get("fallback_to")
        .and_then(|value| value.as_str())
        .map(|fallback| fallback == node_id)
        .unwrap_or(false);

    target_match || fallback_match
}

#[cfg(test)]
mod tests {
    use super::{apply_dispatch_policy, filter_dispatch_for_node};
    use serde_json::json;

    #[test]
    fn supported_action_goes_to_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "project_id": "smart-city-campinas",
            "generated_at_utc": "2026-05-19T13:00:00Z",
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "capability": { "decision": "supported", "reason": "ok", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(dispatch["actions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(
            dispatch["blocked_actions"].as_array().map(|v| v.len()),
            Some(0)
        );
        assert_eq!(
            dispatch["deferred_actions"].as_array().map(|v| v.len()),
            Some(0)
        );
    }

    #[test]
    fn unsupported_action_goes_to_blocked_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "capability": { "decision": "unsupported", "reason": "no", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(
            dispatch["blocked_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
    }

    #[test]
    fn unknown_action_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "capability": { "decision": "unknown", "reason": "later", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(
            dispatch["deferred_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
    }

    #[test]
    fn missing_capability_metadata_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01"
                }
            ]
        }));

        assert_eq!(
            dispatch["deferred_actions"][0]["capability"]["reason"].as_str(),
            Some("capability metadata missing")
        );
    }

    #[test]
    fn no_actions_produces_empty_arrays_and_zero_counts() {
        let dispatch = apply_dispatch_policy(&json!({
            "project_id": "smart-city-campinas"
        }));

        assert_eq!(dispatch["actions"].as_array().map(|v| v.len()), Some(0));
        assert_eq!(
            dispatch["blocked_actions"].as_array().map(|v| v.len()),
            Some(0)
        );
        assert_eq!(
            dispatch["deferred_actions"].as_array().map(|v| v.len()),
            Some(0)
        );
        assert_eq!(dispatch["summary"]["total_actions"].as_u64(), Some(0));
    }

    #[test]
    fn original_action_fields_are_preserved() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "artifact": "chirpstack/chirpstack:latest",
                    "ai_context": { "module_id": "chirpstack-primary" },
                    "capability": { "decision": "supported", "reason": "ok", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(
            dispatch["actions"][0]["artifact"].as_str(),
            Some("chirpstack/chirpstack:latest")
        );
        assert_eq!(
            dispatch["actions"][0]["ai_context"]["module_id"].as_str(),
            Some("chirpstack-primary")
        );
    }

    #[test]
    fn summary_counts_are_correct() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "capability": { "decision": "supported", "reason": "ok", "node_id": "fog-01", "source": "registered" }
                },
                {
                    "action_id": "a2",
                    "target_node": "fog-01",
                    "capability": { "decision": "unsupported", "reason": "no", "node_id": "fog-01", "source": "registered" }
                },
                {
                    "action_id": "a3",
                    "target_node": "fog-01",
                    "capability": { "decision": "unknown", "reason": "later", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(dispatch["summary"]["total_actions"].as_u64(), Some(3));
        assert_eq!(
            dispatch["summary"]["dispatchable_actions"].as_u64(),
            Some(1)
        );
        assert_eq!(dispatch["summary"]["blocked_actions"].as_u64(), Some(1));
        assert_eq!(dispatch["summary"]["deferred_actions"].as_u64(), Some(1));
    }

    #[test]
    fn node_scoped_filter_returns_actions_blocked_and_deferred_separately() {
        let dispatch = json!({
            "project_id": "smart-city-campinas",
            "dispatch_version": "0.2.0",
            "generated_at_utc": "2026-05-19T13:00:00Z",
            "dispatch_policy": { "mode": "capability_gate" },
            "summary": { "total_actions": 4 },
            "actions": [
                { "action_id": "a1", "target_node": "fog-01" },
                { "action_id": "a2", "target_node": "fog-02" }
            ],
            "blocked_actions": [
                { "action_id": "b1", "target_node": "fog-01" }
            ],
            "deferred_actions": [
                { "action_id": "d1", "fallback_to": "fog-01" }
            ]
        });

        let scoped = filter_dispatch_for_node(&dispatch, "smart-city-campinas", "fog-01");

        assert_eq!(scoped["actions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(
            scoped["blocked_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            scoped["deferred_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
        assert_eq!(scoped["summary"]["assigned_actions"].as_u64(), Some(1));
        assert_eq!(
            scoped["summary"]["assigned_blocked_actions"].as_u64(),
            Some(1)
        );
        assert_eq!(
            scoped["summary"]["assigned_deferred_actions"].as_u64(),
            Some(1)
        );
    }

    #[test]
    fn node_scoped_filter_excludes_unrelated_nodes() {
        let dispatch = json!({
            "actions": [
                { "action_id": "a1", "target_node": "fog-02" }
            ],
            "blocked_actions": [
                { "action_id": "b1", "target_node": "fog-03" }
            ],
            "deferred_actions": [
                { "action_id": "d1", "fallback_to": "fog-04" }
            ]
        });

        let scoped = filter_dispatch_for_node(&dispatch, "smart-city-campinas", "fog-01");

        assert_eq!(scoped["actions"].as_array().map(|v| v.len()), Some(0));
        assert_eq!(
            scoped["blocked_actions"].as_array().map(|v| v.len()),
            Some(0)
        );
        assert_eq!(
            scoped["deferred_actions"].as_array().map(|v| v.len()),
            Some(0)
        );
    }
}
