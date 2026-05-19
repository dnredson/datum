use std::collections::BTreeMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const STATUS_DRY_RUN: &str = "dry_run";
const STATUS_BLOCKED: &str = "blocked";
const STATUS_SKIPPED: &str = "skipped";
const STATUS_REJECTED: &str = "rejected";
const STATUS_WOULD_EXECUTE: &str = "would_execute";
const STATUS_ALREADY_RUNNING: &str = "already_running";
const STATUS_STARTED: &str = "started";
const STATUS_NOT_INSTALLED: &str = "not_installed";
const STATUS_MISSING_LOCAL_MAPPING: &str = "missing_local_mapping";
const STATUS_DOCKER_UNAVAILABLE: &str = "docker_unavailable";
const STATUS_FAILED: &str = "failed";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionExecutionReport {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub dispatch_version: Option<String>,
    #[serde(default)]
    pub action_plan_id: Option<String>,
    #[serde(default)]
    pub dispatch_generated_at_utc: Option<String>,
    #[serde(default)]
    pub reported_at_utc: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub results: Vec<ActionExecutionResult>,
    #[serde(default)]
    pub summary: Option<ActionExecutionSummary>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionExecutionResult {
    #[serde(default)]
    pub action_id: String,
    #[serde(default)]
    pub action_kind: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub target_node: Option<String>,
    #[serde(default)]
    pub assigned_node: Option<String>,
    #[serde(default)]
    pub execution_status: String,
    #[serde(default)]
    pub validation_status: Option<String>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub started_at_utc: Option<String>,
    #[serde(default)]
    pub finished_at_utc: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub capability: Option<Value>,
    #[serde(default)]
    pub ai_context: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ActionExecutionSummary {
    pub total_results: u64,
    #[serde(default)]
    pub dry_run: Option<u64>,
    #[serde(default)]
    pub blocked: Option<u64>,
    #[serde(default)]
    pub skipped: Option<u64>,
    #[serde(default)]
    pub rejected: Option<u64>,
    #[serde(default)]
    pub would_execute: Option<u64>,
    #[serde(default)]
    pub already_running: Option<u64>,
    #[serde(default)]
    pub started: Option<u64>,
    #[serde(default)]
    pub not_installed: Option<u64>,
    #[serde(default)]
    pub missing_local_mapping: Option<u64>,
    #[serde(default)]
    pub docker_unavailable: Option<u64>,
    #[serde(default)]
    pub failed: Option<u64>,
    #[serde(default)]
    pub side_effects_performed: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn report_id_for(project_id: &str, node_id: &str) -> String {
    format!(
        "project:{}:node:{}:reported:{}",
        project_id,
        node_id,
        Uuid::new_v4()
    )
}

fn increment(value: &mut Option<u64>) {
    *value = Some(value.unwrap_or(0) + 1);
}

fn is_valid_execution_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_DRY_RUN
            | STATUS_BLOCKED
            | STATUS_SKIPPED
            | STATUS_REJECTED
            | STATUS_WOULD_EXECUTE
            | STATUS_ALREADY_RUNNING
            | STATUS_STARTED
            | STATUS_NOT_INSTALLED
            | STATUS_MISSING_LOCAL_MAPPING
            | STATUS_DOCKER_UNAVAILABLE
            | STATUS_FAILED
    )
}

pub fn normalize_action_execution_report(
    project_id: &str,
    node_id: &str,
    mut input: ActionExecutionReport,
) -> ActionExecutionReport {
    let reported_at_utc = Utc::now().to_rfc3339();

    input.project_id = project_id.to_string();
    input.node_id = node_id.to_string();
    input.reported_at_utc = reported_at_utc;
    input.report_id = Some(report_id_for(project_id, node_id));

    if input
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        input.source = Some("client".to_string());
    }

    input.summary = Some(compute_action_execution_summary(&input.results));
    input
}

pub fn validate_action_execution_report(report: &ActionExecutionReport) -> Result<(), String> {
    if report.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }

    if report.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if report.reported_at_utc.trim().is_empty() {
        return Err("reported_at_utc must not be empty".to_string());
    }

    for (idx, result) in report.results.iter().enumerate() {
        if result.action_id.trim().is_empty() {
            return Err(format!("results[{idx}].action_id must not be empty"));
        }

        if !is_valid_execution_status(result.execution_status.trim()) {
            return Err(format!(
                "results[{idx}].execution_status is invalid: {}",
                result.execution_status
            ));
        }

        if result.side_effects_performed == Some(true)
            && result.execution_status.trim() != STATUS_STARTED
        {
            return Err(format!(
                "results[{idx}].side_effects_performed=true requires execution_status='started'"
            ));
        }
    }

    Ok(())
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
            STATUS_DRY_RUN => increment(&mut summary.dry_run),
            STATUS_BLOCKED => increment(&mut summary.blocked),
            STATUS_SKIPPED => increment(&mut summary.skipped),
            STATUS_REJECTED => increment(&mut summary.rejected),
            STATUS_WOULD_EXECUTE => increment(&mut summary.would_execute),
            STATUS_ALREADY_RUNNING => increment(&mut summary.already_running),
            STATUS_STARTED => increment(&mut summary.started),
            STATUS_NOT_INSTALLED => increment(&mut summary.not_installed),
            STATUS_MISSING_LOCAL_MAPPING => increment(&mut summary.missing_local_mapping),
            STATUS_DOCKER_UNAVAILABLE => increment(&mut summary.docker_unavailable),
            STATUS_FAILED => increment(&mut summary.failed),
            _ => {}
        }

        if result.side_effects_performed == Some(true) {
            increment(&mut summary.side_effects_performed);
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn result(status: &str) -> ActionExecutionResult {
        ActionExecutionResult {
            action_id: "a1".to_string(),
            execution_status: status.to_string(),
            ..ActionExecutionResult::default()
        }
    }

    fn base_report() -> ActionExecutionReport {
        ActionExecutionReport {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            reported_at_utc: "2026-05-19T13:00:00Z".to_string(),
            ..ActionExecutionReport::default()
        }
    }

    #[test]
    fn summary_counts_dry_run() {
        let summary = compute_action_execution_summary(&[result(STATUS_DRY_RUN)]);
        assert_eq!(summary.total_results, 1);
        assert_eq!(summary.dry_run, Some(1));
    }

    #[test]
    fn summary_counts_blocked() {
        let summary = compute_action_execution_summary(&[result(STATUS_BLOCKED)]);
        assert_eq!(summary.blocked, Some(1));
    }

    #[test]
    fn summary_counts_would_execute() {
        let summary = compute_action_execution_summary(&[result(STATUS_WOULD_EXECUTE)]);
        assert_eq!(summary.would_execute, Some(1));
    }

    #[test]
    fn summary_counts_started_and_side_effects_performed() {
        let summary = compute_action_execution_summary(&[ActionExecutionResult {
            action_id: "a1".to_string(),
            execution_status: STATUS_STARTED.to_string(),
            side_effects_performed: Some(true),
            ..ActionExecutionResult::default()
        }]);

        assert_eq!(summary.started, Some(1));
        assert_eq!(summary.side_effects_performed, Some(1));
    }

    #[test]
    fn validation_accepts_empty_results() {
        assert!(validate_action_execution_report(&base_report()).is_ok());
    }

    #[test]
    fn validation_rejects_missing_action_id() {
        let mut report = base_report();
        report.results = vec![ActionExecutionResult {
            action_id: "".to_string(),
            execution_status: STATUS_DRY_RUN.to_string(),
            ..ActionExecutionResult::default()
        }];

        assert!(validate_action_execution_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_unknown_execution_status() {
        let mut report = base_report();
        report.results = vec![ActionExecutionResult {
            action_id: "a1".to_string(),
            execution_status: "explode".to_string(),
            ..ActionExecutionResult::default()
        }];

        assert!(validate_action_execution_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_side_effects_for_non_started_status() {
        let mut report = base_report();
        report.results = vec![ActionExecutionResult {
            action_id: "a1".to_string(),
            execution_status: STATUS_DRY_RUN.to_string(),
            side_effects_performed: Some(true),
            ..ActionExecutionResult::default()
        }];

        assert!(validate_action_execution_report(&report).is_err());
    }

    #[test]
    fn normalization_overrides_project_id_and_node_id() {
        let normalized = normalize_action_execution_report(
            "project-a",
            "node-b",
            ActionExecutionReport {
                project_id: "wrong".to_string(),
                node_id: "wrong".to_string(),
                reported_at_utc: "old".to_string(),
                ..ActionExecutionReport::default()
            },
        );

        assert_eq!(normalized.project_id, "project-a");
        assert_eq!(normalized.node_id, "node-b");
    }

    #[test]
    fn normalization_defaults_source_to_client() {
        let normalized = normalize_action_execution_report(
            "project-a",
            "node-b",
            ActionExecutionReport {
                reported_at_utc: "old".to_string(),
                ..ActionExecutionReport::default()
            },
        );

        assert_eq!(normalized.source.as_deref(), Some("client"));
    }

    #[test]
    fn normalization_recomputes_summary() {
        let normalized = normalize_action_execution_report(
            "project-a",
            "node-b",
            ActionExecutionReport {
                reported_at_utc: "old".to_string(),
                summary: Some(ActionExecutionSummary {
                    total_results: 99,
                    dry_run: Some(99),
                    ..ActionExecutionSummary::default()
                }),
                results: vec![ActionExecutionResult {
                    action_id: "a1".to_string(),
                    execution_status: STATUS_DRY_RUN.to_string(),
                    metadata: Some(json!({ "kept": true })),
                    ..ActionExecutionResult::default()
                }],
                ..ActionExecutionReport::default()
            },
        );

        assert_eq!(
            normalized.summary,
            Some(ActionExecutionSummary {
                total_results: 1,
                dry_run: Some(1),
                ..ActionExecutionSummary::default()
            })
        );
    }
}
