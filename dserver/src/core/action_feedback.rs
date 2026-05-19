use std::collections::HashMap;

use serde_json::{json, Value};

use crate::core::action_results::ActionExecutionReport;

pub fn annotate_action_plan_with_latest_results(
    mut action_plan: Value,
    latest_reports_by_node: &HashMap<String, ActionExecutionReport>,
) -> Value {
    let Some(actions) = action_plan
        .get_mut("actions")
        .and_then(|v| v.as_array_mut())
    else {
        return action_plan;
    };

    let mut matched = 0_u64;
    let mut no_matching_action_result = 0_u64;
    let mut no_report_for_node = 0_u64;
    let mut missing_action_id = 0_u64;

    for action in actions.iter_mut() {
        let feedback = build_action_feedback(action, latest_reports_by_node);
        match feedback
            .get("status")
            .and_then(|value| value.as_str())
            .unwrap_or("no_report_for_node")
        {
            "matched" => matched += 1,
            "no_matching_action_result" => no_matching_action_result += 1,
            "missing_action_id" => missing_action_id += 1,
            _ => no_report_for_node += 1,
        }

        if let Some(action_obj) = action.as_object_mut() {
            action_obj.insert("feedback".to_string(), feedback);
        }
    }

    if let Some(plan_obj) = action_plan.as_object_mut() {
        plan_obj.insert(
            "feedback_summary".to_string(),
            json!({
                "matched": matched,
                "no_matching_action_result": no_matching_action_result,
                "no_report_for_node": no_report_for_node,
                "missing_action_id": missing_action_id
            }),
        );
    }

    action_plan
}

fn build_action_feedback(
    action: &Value,
    latest_reports_by_node: &HashMap<String, ActionExecutionReport>,
) -> Value {
    let node_id = action
        .get("target_node")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let action_id = action
        .get("action_id")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let Some(action_id) = action_id else {
        return json!({
            "source": "latest_action_results",
            "status": "missing_action_id",
            "message": "action has no action_id",
            "node_id": node_id
        });
    };

    let Some(node_id) = node_id else {
        return json!({
            "source": "latest_action_results",
            "status": "no_report_for_node",
            "message": "action has no target node",
            "node_id": Value::Null
        });
    };

    let Some(report) = latest_reports_by_node.get(&node_id) else {
        return json!({
            "source": "latest_action_results",
            "status": "no_report_for_node",
            "message": "no action result report available for target node",
            "node_id": node_id
        });
    };

    if let Some(result) = report
        .results
        .iter()
        .find(|result| result.action_id == action_id)
    {
        return json!({
            "source": "latest_action_results",
            "status": "matched",
            "execution_status": result.execution_status,
            "validation_status": result.validation_status,
            "side_effects_performed": result.side_effects_performed,
            "message": result.message,
            "reported_at_utc": report.reported_at_utc,
            "report_id": report.report_id,
            "node_id": node_id
        });
    }

    json!({
        "source": "latest_action_results",
        "status": "no_matching_action_result",
        "message": "no matching action result found in latest report for target node",
        "reported_at_utc": report.reported_at_utc,
        "report_id": report.report_id,
        "node_id": node_id
    })
}

#[cfg(test)]
mod tests {
    use super::annotate_action_plan_with_latest_results;
    use crate::core::action_results::{ActionExecutionReport, ActionExecutionResult};
    use serde_json::{json, Value};
    use std::collections::HashMap;

    fn report_for(node_id: &str, action_id: &str, execution_status: &str) -> ActionExecutionReport {
        ActionExecutionReport {
            project_id: "smart-city-campinas".to_string(),
            node_id: node_id.to_string(),
            report_id: Some(format!("report:{node_id}")),
            reported_at_utc: "2026-05-19T14:00:00Z".to_string(),
            results: vec![ActionExecutionResult {
                action_id: action_id.to_string(),
                execution_status: execution_status.to_string(),
                validation_status: Some("valid".to_string()),
                side_effects_performed: Some(execution_status == "started"),
                message: Some(format!("status={execution_status}")),
                ..ActionExecutionResult::default()
            }],
            ..ActionExecutionReport::default()
        }
    }

    fn plan_with(actions: Vec<Value>) -> Value {
        json!({
            "project_id": "smart-city-campinas",
            "actions": actions
        })
    }

    #[test]
    fn matching_action_id_adds_feedback_status_matched() {
        let mut reports = HashMap::new();
        reports.insert(
            "fog-01".to_string(),
            report_for("fog-01", "gap:1", "dry_run"),
        );

        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "action_id": "gap:1",
                "target_node": "fog-01"
            })]),
            &reports,
        );

        assert_eq!(
            plan["actions"][0]["feedback"]["status"].as_str(),
            Some("matched")
        );
    }

    #[test]
    fn matched_feedback_includes_execution_status_report_id_reported_at_and_node_id() {
        let mut reports = HashMap::new();
        reports.insert(
            "fog-01".to_string(),
            report_for("fog-01", "gap:1", "started"),
        );

        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "action_id": "gap:1",
                "target_node": "fog-01"
            })]),
            &reports,
        );

        let feedback = &plan["actions"][0]["feedback"];
        assert_eq!(feedback["execution_status"].as_str(), Some("started"));
        assert_eq!(feedback["report_id"].as_str(), Some("report:fog-01"));
        assert_eq!(
            feedback["reported_at_utc"].as_str(),
            Some("2026-05-19T14:00:00Z")
        );
        assert_eq!(feedback["node_id"].as_str(), Some("fog-01"));
    }

    #[test]
    fn latest_report_exists_but_no_matching_action_id_adds_no_matching_action_result() {
        let mut reports = HashMap::new();
        reports.insert(
            "fog-01".to_string(),
            report_for("fog-01", "gap:other", "dry_run"),
        );

        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "action_id": "gap:1",
                "target_node": "fog-01"
            })]),
            &reports,
        );

        assert_eq!(
            plan["actions"][0]["feedback"]["status"].as_str(),
            Some("no_matching_action_result")
        );
    }

    #[test]
    fn no_report_for_target_node_adds_no_report_for_node() {
        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "action_id": "gap:1",
                "target_node": "fog-01"
            })]),
            &HashMap::new(),
        );

        assert_eq!(
            plan["actions"][0]["feedback"]["status"].as_str(),
            Some("no_report_for_node")
        );
    }

    #[test]
    fn missing_action_id_adds_missing_action_id() {
        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "target_node": "fog-01"
            })]),
            &HashMap::new(),
        );

        assert_eq!(
            plan["actions"][0]["feedback"]["status"].as_str(),
            Some("missing_action_id")
        );
    }

    #[test]
    fn feedback_summary_counts_are_correct() {
        let mut reports = HashMap::new();
        reports.insert(
            "fog-01".to_string(),
            report_for("fog-01", "gap:1", "dry_run"),
        );
        reports.insert(
            "fog-02".to_string(),
            report_for("fog-02", "gap:other", "failed"),
        );

        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![
                json!({ "action_id": "gap:1", "target_node": "fog-01" }),
                json!({ "action_id": "gap:2", "target_node": "fog-02" }),
                json!({ "action_id": "gap:3", "target_node": "fog-03" }),
                json!({ "target_node": "fog-04" }),
            ]),
            &reports,
        );

        assert_eq!(plan["feedback_summary"]["matched"].as_u64(), Some(1));
        assert_eq!(
            plan["feedback_summary"]["no_matching_action_result"].as_u64(),
            Some(1)
        );
        assert_eq!(
            plan["feedback_summary"]["no_report_for_node"].as_u64(),
            Some(1)
        );
        assert_eq!(
            plan["feedback_summary"]["missing_action_id"].as_u64(),
            Some(1)
        );
    }

    #[test]
    fn existing_capability_metadata_is_preserved() {
        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "action_id": "gap:1",
                "target_node": "fog-01",
                "capability": { "decision": "supported" }
            })]),
            &HashMap::new(),
        );

        assert_eq!(
            plan["actions"][0]["capability"]["decision"].as_str(),
            Some("supported")
        );
    }

    #[test]
    fn existing_ai_context_is_preserved() {
        let plan = annotate_action_plan_with_latest_results(
            plan_with(vec![json!({
                "action_id": "gap:1",
                "target_node": "fog-01",
                "ai_context": { "module_id": "chirpstack-primary" }
            })]),
            &HashMap::new(),
        );

        assert_eq!(
            plan["actions"][0]["ai_context"]["module_id"].as_str(),
            Some("chirpstack-primary")
        );
    }
}
