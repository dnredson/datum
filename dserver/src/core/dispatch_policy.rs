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
    let mut feedback_deferred_actions = 0_usize;
    let mut feedback_blocked_actions = 0_usize;

    for action in actions {
        let normalized = ensure_capability_metadata(action);
        let decision = decide_dispatch_bucket(normalized);

        if decision.feedback_policy {
            match decision.bucket {
                "blocked_actions" => feedback_blocked_actions += 1,
                "deferred_actions" => feedback_deferred_actions += 1,
                _ => {}
            }
        }

        match decision.bucket {
            "actions" => dispatchable_actions.push(decision.action),
            "blocked_actions" => blocked_actions.push(decision.action),
            _ => deferred_actions.push(decision.action),
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
            "policy_version": "0.2.0",
            "mode": "capability_feedback_gate",
            "supported_actions_dispatched": true,
            "unsupported_actions_blocked": true,
            "unknown_actions_deferred": true,
            "feedback_aware": true
        },
        "summary": {
            "total_actions": action_plan
                .get("actions")
                .and_then(|value| value.as_array())
                .map(|value| value.len())
                .unwrap_or(0),
            "dispatchable_actions": dispatchable_actions.len(),
            "blocked_actions": blocked_actions.len(),
            "deferred_actions": deferred_actions.len(),
            "feedback_deferred_actions": feedback_deferred_actions,
            "feedback_blocked_actions": feedback_blocked_actions
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

struct DispatchBucketDecision {
    action: Value,
    bucket: &'static str,
    feedback_policy: bool,
}

fn decide_dispatch_bucket(mut action: Value) -> DispatchBucketDecision {
    let capability_decision = action
        .get("capability")
        .and_then(|value| value.get("decision"))
        .and_then(|value| value.as_str())
        .unwrap_or("unknown")
        .to_string();
    let feedback_status = action
        .get("feedback")
        .and_then(|value| value.get("status"))
        .and_then(|value| value.as_str())
        .unwrap_or("no_report_for_node")
        .to_string();
    let feedback_execution_status = action
        .get("feedback")
        .and_then(|value| value.get("execution_status"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string);

    let (bucket, policy_decision, reason, feedback_policy): (&str, &str, String, bool) =
        match capability_decision.as_str() {
        "unsupported" => (
            "blocked_actions",
            "blocked_capability_unsupported",
            action
                .get("capability")
                .and_then(|value| value.get("reason"))
                .and_then(|value| value.as_str())
                .unwrap_or("target node capabilities do not support this action")
                .to_string(),
            false,
        ),
        "supported" => match (feedback_status.as_str(), feedback_execution_status.as_deref()) {
            ("matched", Some("started")) => (
                "deferred_actions",
                "deferred_recently_started",
                "latest result already started this action".to_string(),
                true,
            ),
            ("matched", Some("already_running")) => (
                "deferred_actions",
                "deferred_already_running",
                "latest result indicates target is already running".to_string(),
                true,
            ),
            ("matched", Some("blocked")) => (
                "deferred_actions",
                "deferred_client_blocked",
                "latest result was blocked by client-side configuration".to_string(),
                true,
            ),
            ("matched", Some("missing_local_mapping")) => (
                "blocked_actions",
                "blocked_missing_local_mapping",
                "client lacks trusted local mapping for this action".to_string(),
                true,
            ),
            ("matched", Some("not_installed")) => (
                "deferred_actions",
                "deferred_installation_required",
                "target container/service is not installed locally; installation planning is required".to_string(),
                true,
            ),
            ("matched", Some("docker_unavailable")) => (
                "deferred_actions",
                "deferred_runtime_unavailable",
                "client reported Docker unavailable".to_string(),
                true,
            ),
            ("matched", Some("failed")) => (
                "deferred_actions",
                "deferred_failed_without_retry_policy",
                "latest execution failed; retry policy not implemented yet".to_string(),
                true,
            ),
            ("matched", Some("dry_run")) => (
                "actions",
                "dispatch_after_dry_run",
                "latest result was dry-run only".to_string(),
                true,
            ),
            ("matched", Some("would_execute")) => (
                "actions",
                "dispatch_after_would_execute",
                "latest result did not perform side effects".to_string(),
                true,
            ),
            ("no_matching_action_result", _) => (
                "actions",
                "dispatch_no_matching_action_result",
                "latest report for target node did not include this action".to_string(),
                false,
            ),
            ("no_report_for_node", _) => (
                "actions",
                "dispatch_no_report_for_node",
                "no action result report available for target node".to_string(),
                false,
            ),
            ("missing_action_id", _) => (
                "actions",
                "dispatch_missing_action_id_feedback",
                "action feedback could not match because action_id is missing".to_string(),
                false,
            ),
            _ => (
                "actions",
                "dispatch_without_feedback_constraint",
                "no feedback-based dispatch constraint applied".to_string(),
                false,
            ),
        },
        _ => (
            "deferred_actions",
            "deferred_capability_unknown",
            action
                .get("capability")
                .and_then(|value| value.get("reason"))
                .and_then(|value| value.as_str())
                .unwrap_or("capability decision is unknown")
                .to_string(),
            false,
        ),
    };

    if let Some(obj) = action.as_object_mut() {
        obj.insert(
            "dispatch_decision".to_string(),
            json!({
                "bucket": bucket,
                "policy_decision": policy_decision,
                "reason": reason,
                "capability_decision": capability_decision,
                "feedback_status": feedback_status,
                "feedback_execution_status": feedback_execution_status
            }),
        );
    }

    DispatchBucketDecision {
        action,
        bucket,
        feedback_policy,
    }
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

    fn supported_action_with_feedback(execution_status: Option<&str>) -> serde_json::Value {
        let mut action = json!({
            "action_id": "a1",
            "target_node": "fog-01",
            "artifact": "chirpstack/chirpstack:latest",
            "ai_context": { "module_id": "chirpstack-primary" },
            "capability": { "decision": "supported", "reason": "ok", "node_id": "fog-01", "source": "registered" }
        });

        if let Some(status) = execution_status {
            action["feedback"] = json!({
                "source": "latest_action_results",
                "status": "matched",
                "execution_status": status
            });
        }

        action
    }

    #[test]
    fn supported_action_with_no_feedback_remains_in_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(None)]
        }));

        assert_eq!(dispatch["actions"].as_array().map(|v| v.len()), Some(1));
        assert_eq!(
            dispatch["actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("dispatch_no_report_for_node")
        );
    }

    #[test]
    fn supported_action_with_feedback_dry_run_remains_in_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("dry_run"))]
        }));

        assert_eq!(
            dispatch["actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("dispatch_after_dry_run")
        );
    }

    #[test]
    fn supported_action_with_feedback_would_execute_remains_in_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("would_execute"))]
        }));

        assert_eq!(
            dispatch["actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("dispatch_after_would_execute")
        );
    }

    #[test]
    fn supported_action_with_feedback_started_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("started"))]
        }));

        assert_eq!(
            dispatch["deferred_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("deferred_recently_started")
        );
    }

    #[test]
    fn supported_action_with_feedback_already_running_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("already_running"))]
        }));

        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("deferred_already_running")
        );
    }

    #[test]
    fn supported_action_with_feedback_blocked_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("blocked"))]
        }));

        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("deferred_client_blocked")
        );
    }

    #[test]
    fn supported_action_with_feedback_missing_local_mapping_goes_to_blocked_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("missing_local_mapping"))]
        }));

        assert_eq!(
            dispatch["blocked_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            dispatch["blocked_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("blocked_missing_local_mapping")
        );
    }

    #[test]
    fn supported_action_with_feedback_not_installed_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("not_installed"))]
        }));

        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("deferred_installation_required")
        );
    }

    #[test]
    fn supported_action_with_feedback_docker_unavailable_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("docker_unavailable"))]
        }));

        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("deferred_runtime_unavailable")
        );
    }

    #[test]
    fn supported_action_with_feedback_failed_goes_to_deferred_actions() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("failed"))]
        }));

        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["policy_decision"].as_str(),
            Some("deferred_failed_without_retry_policy")
        );
    }

    #[test]
    fn unsupported_capability_still_goes_to_blocked_actions_even_if_feedback_says_dry_run() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "feedback": { "status": "matched", "execution_status": "dry_run" },
                    "capability": { "decision": "unsupported", "reason": "no", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(
            dispatch["blocked_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            dispatch["blocked_actions"][0]["dispatch_decision"]["capability_decision"].as_str(),
            Some("unsupported")
        );
    }

    #[test]
    fn unknown_capability_still_goes_to_deferred_actions_even_if_feedback_says_dry_run() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                {
                    "action_id": "a1",
                    "target_node": "fog-01",
                    "feedback": { "status": "matched", "execution_status": "dry_run" },
                    "capability": { "decision": "unknown", "reason": "later", "node_id": "fog-01", "source": "registered" }
                }
            ]
        }));

        assert_eq!(
            dispatch["deferred_actions"].as_array().map(|v| v.len()),
            Some(1)
        );
        assert_eq!(
            dispatch["deferred_actions"][0]["dispatch_decision"]["capability_decision"].as_str(),
            Some("unknown")
        );
    }

    #[test]
    fn dispatch_decision_metadata_is_present_and_correct() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("started"))]
        }));

        let decision = &dispatch["deferred_actions"][0]["dispatch_decision"];
        assert_eq!(decision["bucket"].as_str(), Some("deferred_actions"));
        assert_eq!(
            decision["policy_decision"].as_str(),
            Some("deferred_recently_started")
        );
        assert_eq!(
            decision["reason"].as_str(),
            Some("latest result already started this action")
        );
        assert_eq!(decision["capability_decision"].as_str(), Some("supported"));
        assert_eq!(decision["feedback_status"].as_str(), Some("matched"));
        assert_eq!(
            decision["feedback_execution_status"].as_str(),
            Some("started")
        );
    }

    #[test]
    fn summary_counts_are_correct() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [
                supported_action_with_feedback(Some("dry_run")),
                supported_action_with_feedback(Some("missing_local_mapping")),
                supported_action_with_feedback(Some("failed"))
            ]
        }));

        assert_eq!(dispatch["summary"]["total_actions"].as_u64(), Some(3));
        assert_eq!(
            dispatch["summary"]["dispatchable_actions"].as_u64(),
            Some(1)
        );
        assert_eq!(dispatch["summary"]["blocked_actions"].as_u64(), Some(1));
        assert_eq!(dispatch["summary"]["deferred_actions"].as_u64(), Some(1));
        assert_eq!(
            dispatch["summary"]["feedback_blocked_actions"].as_u64(),
            Some(1)
        );
        assert_eq!(
            dispatch["summary"]["feedback_deferred_actions"].as_u64(),
            Some(1)
        );
    }

    #[test]
    fn existing_ai_context_capability_and_feedback_fields_are_preserved() {
        let dispatch = apply_dispatch_policy(&json!({
            "actions": [supported_action_with_feedback(Some("dry_run"))]
        }));

        assert_eq!(
            dispatch["actions"][0]["artifact"].as_str(),
            Some("chirpstack/chirpstack:latest")
        );
        assert_eq!(
            dispatch["actions"][0]["ai_context"]["module_id"].as_str(),
            Some("chirpstack-primary")
        );
        assert_eq!(
            dispatch["actions"][0]["feedback"]["status"].as_str(),
            Some("matched")
        );
        assert_eq!(
            dispatch["actions"][0]["feedback"]["execution_status"].as_str(),
            Some("dry_run")
        );
        assert_eq!(
            dispatch["actions"][0]["capability"]["decision"].as_str(),
            Some("supported")
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
    fn node_scoped_filter_returns_actions_blocked_and_deferred_separately() {
        let dispatch = json!({
            "project_id": "smart-city-campinas",
            "dispatch_version": "0.2.0",
            "generated_at_utc": "2026-05-19T13:00:00Z",
            "dispatch_policy": { "mode": "capability_feedback_gate" },
            "summary": { "total_actions": 4 },
            "actions": [
                { "action_id": "a1", "target_node": "fog-01", "dispatch_decision": { "bucket": "actions" } },
                { "action_id": "a2", "target_node": "fog-02", "dispatch_decision": { "bucket": "actions" } }
            ],
            "blocked_actions": [
                { "action_id": "b1", "target_node": "fog-01", "dispatch_decision": { "bucket": "blocked_actions" } }
            ],
            "deferred_actions": [
                { "action_id": "d1", "fallback_to": "fog-01", "dispatch_decision": { "bucket": "deferred_actions" } }
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
        assert_eq!(
            scoped["blocked_actions"][0]["dispatch_decision"]["bucket"].as_str(),
            Some("blocked_actions")
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
