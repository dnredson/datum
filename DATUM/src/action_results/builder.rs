use crate::{
    action_results::model::{ActionExecutionReport, ActionExecutionResult, ActionExecutionSummary},
    dispatch::{
        executor::{DispatchActionExecutionResult, DispatchExecutionReport},
        model::{DispatchAction, DispatchResponse},
    },
};

fn increment(value: &mut Option<u64>) {
    *value = Some(value.unwrap_or(0) + 1);
}

pub fn compute_action_execution_summary(
    results: &[ActionExecutionResult],
) -> ActionExecutionSummary {
    let mut summary = ActionExecutionSummary {
        total_results: results.len() as u64,
        ..ActionExecutionSummary::default()
    };

    for result in results {
        match result.execution_status.as_str() {
            "dry_run" => increment(&mut summary.dry_run),
            "blocked" => increment(&mut summary.blocked),
            "skipped" => increment(&mut summary.skipped),
            "rejected" => increment(&mut summary.rejected),
            "would_execute" => increment(&mut summary.would_execute),
            "already_running" => increment(&mut summary.already_running),
            "started" => increment(&mut summary.started),
            "not_installed" => increment(&mut summary.not_installed),
            "missing_local_mapping" => increment(&mut summary.missing_local_mapping),
            "docker_unavailable" => increment(&mut summary.docker_unavailable),
            "failed" => increment(&mut summary.failed),
            _ => {}
        }

        if result.side_effects_performed == Some(true) {
            increment(&mut summary.side_effects_performed);
        }
    }

    summary
}

pub fn build_action_execution_report_from_dispatch(
    dispatch_response: &DispatchResponse,
    execution_report: &DispatchExecutionReport,
    project_id: &str,
    node_id: &str,
) -> ActionExecutionReport {
    let results = execution_report
        .action_results
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let action = matching_dispatch_action(dispatch_response, item, index);

            ActionExecutionResult {
                action_id: item
                    .action_id
                    .clone()
                    .or_else(|| action.and_then(|a| a.action_id.clone()))
                    .unwrap_or_else(|| format!("unknown-action-{index}")),
                action_kind: item
                    .action_kind
                    .clone()
                    .or_else(|| action.and_then(|a| a.action_kind.clone())),
                runtime: action.and_then(|a| a.runtime.clone()),
                target_node: action.and_then(|a| a.target_node.clone()),
                assigned_node: action.and_then(|a| a.assigned_node.clone()),
                execution_status: item.execution_status.clone(),
                validation_status: Some(item.validation_status.clone()),
                side_effects_performed: match item.side_effects_performed {
                    Some(value) => Some(value),
                    None if item.execution_status == "started" => Some(true),
                    _ => None,
                },
                message: Some(item.message.clone()),
                started_at_utc: None,
                finished_at_utc: None,
                error: (item.execution_status == "failed").then(|| item.message.clone()),
                artifact: action.and_then(|a| a.artifact.clone()),
                capability: action.and_then(|a| a.capability.clone()),
                ai_context: action.and_then(|a| a.ai_context.clone()),
                metadata: None,
                extra: Default::default(),
            }
        })
        .collect::<Vec<_>>();

    let summary = compute_action_execution_summary(&results);
    let dispatch_generated_at_utc = dispatch_response
        .generated_at_utc
        .clone()
        .or_else(|| {
            value_to_string(
                dispatch_response
                    .extra
                    .get("source_action_plan_generated_at_utc"),
            )
        })
        .or_else(|| {
            value_to_string(
                dispatch_response
                    .extra
                    .get("source_dispatch_generated_at_utc"),
            )
        });
    let action_plan_id = value_to_string(dispatch_response.extra.get("action_plan_id"))
        .or_else(|| value_to_string(dispatch_response.extra.get("source_action_plan_id")));

    ActionExecutionReport {
        project_id: Some(project_id.to_string()),
        node_id: Some(node_id.to_string()),
        report_id: None,
        dispatch_version: dispatch_response.dispatch_version.clone(),
        action_plan_id,
        dispatch_generated_at_utc,
        reported_at_utc: None,
        source: Some("client".to_string()),
        results,
        summary: Some(summary),
        metadata: None,
        extra: Default::default(),
    }
}

fn matching_dispatch_action<'a>(
    dispatch_response: &'a DispatchResponse,
    item: &DispatchActionExecutionResult,
    index: usize,
) -> Option<&'a DispatchAction> {
    if let Some(action_id) = item.action_id.as_deref() {
        if let Some(action) = dispatch_response
            .actions
            .iter()
            .find(|action| action.action_id.as_deref() == Some(action_id))
        {
            return Some(action);
        }
    }

    dispatch_response.actions.get(index)
}

fn value_to_string(value: Option<&serde_json::Value>) -> Option<String> {
    value.and_then(|value| value.as_str().map(ToString::to_string))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_action_execution_report_from_dispatch, compute_action_execution_summary};
    use crate::dispatch::{
        executor::{DispatchActionExecutionResult, DispatchExecutionReport},
        model::{DispatchAction, DispatchResponse},
    };

    fn base_dispatch() -> DispatchResponse {
        DispatchResponse {
            dispatch_version: Some("0.2.0".to_string()),
            generated_at_utc: Some("2026-05-19T12:55:53Z".to_string()),
            actions: vec![DispatchAction {
                action_id: Some("gap:1".to_string()),
                action_kind: Some("ensure_container_running".to_string()),
                runtime: Some("container".to_string()),
                artifact: Some("chirpstack/chirpstack:latest".to_string()),
                target_node: Some("fog-01".to_string()),
                assigned_node: Some("fog-01".to_string()),
                capability: Some(json!({ "decision": "supported" })),
                ai_context: Some(json!({ "module_id": "chirpstack-primary" })),
                ..DispatchAction::default()
            }],
            ..DispatchResponse::default()
        }
    }

    fn report_with_status(
        status: &str,
        side_effects_performed: Option<bool>,
    ) -> DispatchExecutionReport {
        DispatchExecutionReport {
            local_node_id: "fog-01".to_string(),
            execute_requested: status != "dry_run",
            side_effects_enabled: status == "started",
            total_actions: 1,
            valid_actions: 1,
            skipped_actions: 0,
            rejected_actions: 0,
            side_effects_performed: side_effects_performed.unwrap_or(false),
            action_results: vec![DispatchActionExecutionResult {
                action_id: Some("gap:1".to_string()),
                action_kind: Some("ensure_container_running".to_string()),
                validation_status: "valid".to_string(),
                execution_status: status.to_string(),
                message: format!("status={status}"),
                side_effects_performed,
            }],
        }
    }

    #[test]
    fn build_report_from_dry_run_dispatch_execution() {
        let report = build_action_execution_report_from_dispatch(
            &base_dispatch(),
            &report_with_status("dry_run", Some(false)),
            "smart-city-campinas",
            "fog-01",
        );

        assert_eq!(report.source.as_deref(), Some("client"));
        assert_eq!(report.results[0].execution_status, "dry_run");
        assert_eq!(report.summary.as_ref().and_then(|s| s.dry_run), Some(1));
    }

    #[test]
    fn build_report_from_blocked_execution() {
        let report = build_action_execution_report_from_dispatch(
            &base_dispatch(),
            &report_with_status("blocked", Some(false)),
            "smart-city-campinas",
            "fog-01",
        );

        assert_eq!(report.results[0].execution_status, "blocked");
        assert_eq!(report.summary.as_ref().and_then(|s| s.blocked), Some(1));
    }

    #[test]
    fn build_report_from_already_running_execution() {
        let report = build_action_execution_report_from_dispatch(
            &base_dispatch(),
            &report_with_status("already_running", Some(false)),
            "smart-city-campinas",
            "fog-01",
        );

        assert_eq!(report.results[0].execution_status, "already_running");
        assert_eq!(
            report.summary.as_ref().and_then(|s| s.already_running),
            Some(1)
        );
    }

    #[test]
    fn build_report_from_started_execution_sets_side_effects_performed_true() {
        let report = build_action_execution_report_from_dispatch(
            &base_dispatch(),
            &report_with_status("started", Some(true)),
            "smart-city-campinas",
            "fog-01",
        );

        assert_eq!(report.results[0].execution_status, "started");
        assert_eq!(report.results[0].side_effects_performed, Some(true));
        assert_eq!(report.summary.as_ref().and_then(|s| s.started), Some(1));
        assert_eq!(
            report
                .summary
                .as_ref()
                .and_then(|s| s.side_effects_performed),
            Some(1)
        );
    }

    #[test]
    fn report_includes_dispatch_version_when_present() {
        let report = build_action_execution_report_from_dispatch(
            &base_dispatch(),
            &report_with_status("dry_run", Some(false)),
            "smart-city-campinas",
            "fog-01",
        );

        assert_eq!(report.dispatch_version.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn report_enriches_result_with_runtime_artifact_ai_context_and_capability() {
        let report = build_action_execution_report_from_dispatch(
            &base_dispatch(),
            &report_with_status("dry_run", Some(false)),
            "smart-city-campinas",
            "fog-01",
        );

        assert_eq!(report.results[0].runtime.as_deref(), Some("container"));
        assert_eq!(
            report.results[0].artifact.as_deref(),
            Some("chirpstack/chirpstack:latest")
        );
        assert_eq!(
            report.results[0]
                .ai_context
                .as_ref()
                .and_then(|v| v.get("module_id"))
                .and_then(|v| v.as_str()),
            Some("chirpstack-primary")
        );
        assert_eq!(
            report.results[0]
                .capability
                .as_ref()
                .and_then(|v| v.get("decision"))
                .and_then(|v| v.as_str()),
            Some("supported")
        );
    }

    #[test]
    fn summary_counts_execution_statuses_correctly() {
        let summary = compute_action_execution_summary(&[
            crate::action_results::model::ActionExecutionResult {
                action_id: "a1".to_string(),
                execution_status: "dry_run".to_string(),
                ..Default::default()
            },
            crate::action_results::model::ActionExecutionResult {
                action_id: "a2".to_string(),
                execution_status: "started".to_string(),
                side_effects_performed: Some(true),
                ..Default::default()
            },
            crate::action_results::model::ActionExecutionResult {
                action_id: "a3".to_string(),
                execution_status: "failed".to_string(),
                ..Default::default()
            },
        ]);

        assert_eq!(summary.total_results, 3);
        assert_eq!(summary.dry_run, Some(1));
        assert_eq!(summary.started, Some(1));
        assert_eq!(summary.failed, Some(1));
        assert_eq!(summary.side_effects_performed, Some(1));
    }
}
