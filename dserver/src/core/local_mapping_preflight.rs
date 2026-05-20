use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const STATUS_PASSED: &str = "passed";
const STATUS_WARNING: &str = "warning";
const STATUS_FAILED: &str = "failed";
const STATUS_EMPTY: &str = "empty";

const ITEM_STATUS_ALREADY_MAPPED: &str = "already_mapped";
const ITEM_STATUS_MISSING_MAPPING: &str = "missing_mapping";
const ITEM_STATUS_AMBIGUOUS: &str = "ambiguous";
const ITEM_STATUS_NOT_APPLICABLE: &str = "not_applicable";
const ITEM_STATUS_WARNING: &str = "warning";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalMappingPreflightReport {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub report_id: String,
    #[serde(default)]
    pub reported_at_utc: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub items: Vec<LocalMappingPreflightItem>,
    #[serde(default)]
    pub summary: LocalMappingPreflightSummary,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub side_effects_performed: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalMappingPreflightItem {
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub planning_id: Option<String>,
    #[serde(default)]
    pub action_id: Option<String>,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub required_step: Option<String>,
    #[serde(default)]
    pub installation_action: Option<String>,
    #[serde(default)]
    pub target_node: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub current_mapping_source: Option<String>,
    #[serde(default)]
    pub current_container_name: Option<String>,
    #[serde(default)]
    pub suggested_mapping_key: Option<String>,
    #[serde(default)]
    pub suggested_artifact: Option<String>,
    #[serde(default)]
    pub source_task: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct LocalMappingPreflightSummary {
    pub total_items: u64,
    pub applicable_items: u64,
    pub already_mapped: u64,
    pub missing_mapping: u64,
    pub ambiguous: u64,
    pub not_applicable: u64,
    pub warnings: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalMappingPreflightReportList {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub reports: Vec<LocalMappingPreflightReport>,
}

fn report_id_for(project_id: &str, node_id: &str) -> String {
    format!(
        "project:{}:node:{}:report:{}",
        project_id,
        node_id,
        Uuid::new_v4()
    )
}

fn is_valid_report_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_PASSED | STATUS_WARNING | STATUS_FAILED | STATUS_EMPTY
    )
}

fn is_valid_item_status(status: &str) -> bool {
    matches!(
        status,
        ITEM_STATUS_ALREADY_MAPPED
            | ITEM_STATUS_MISSING_MAPPING
            | ITEM_STATUS_AMBIGUOUS
            | ITEM_STATUS_NOT_APPLICABLE
            | ITEM_STATUS_WARNING
    )
}

pub fn compute_local_mapping_preflight_summary(
    items: &[LocalMappingPreflightItem],
) -> LocalMappingPreflightSummary {
    let mut summary = LocalMappingPreflightSummary {
        total_items: items.len() as u64,
        ..LocalMappingPreflightSummary::default()
    };

    for item in items {
        match item.status.trim() {
            ITEM_STATUS_ALREADY_MAPPED => {
                summary.already_mapped += 1;
                summary.applicable_items += 1;
            }
            ITEM_STATUS_MISSING_MAPPING => {
                summary.missing_mapping += 1;
                summary.applicable_items += 1;
            }
            ITEM_STATUS_AMBIGUOUS => {
                summary.ambiguous += 1;
                summary.applicable_items += 1;
            }
            ITEM_STATUS_NOT_APPLICABLE => {
                summary.not_applicable += 1;
            }
            ITEM_STATUS_WARNING => {
                summary.warnings += 1;
                summary.applicable_items += 1;
            }
            _ => {}
        }
    }

    summary
}

pub fn normalize_local_mapping_preflight_report(
    project_id: &str,
    node_id: &str,
    mut input: LocalMappingPreflightReport,
) -> LocalMappingPreflightReport {
    input.project_id = project_id.to_string();
    input.node_id = node_id.to_string();
    input.report_id = report_id_for(project_id, node_id);
    input.reported_at_utc = Utc::now().to_rfc3339();

    if input
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        input.source = Some("client".to_string());
    }

    input.summary = compute_local_mapping_preflight_summary(&input.items);
    input
}

pub fn validate_local_mapping_preflight_report(
    report: &LocalMappingPreflightReport,
) -> Result<(), String> {
    if report.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }

    if report.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if report.report_id.trim().is_empty() {
        return Err("report_id must not be empty".to_string());
    }

    if report.reported_at_utc.trim().is_empty() {
        return Err("reported_at_utc must not be empty".to_string());
    }

    if !is_valid_report_status(report.status.trim()) {
        return Err(format!("status is invalid: {}", report.status));
    }

    if report.safe_to_auto_execute {
        return Err("safe_to_auto_execute must be false".to_string());
    }

    if report.side_effects_performed {
        return Err("side_effects_performed must be false".to_string());
    }

    for (idx, item) in report.items.iter().enumerate() {
        if !is_valid_item_status(item.status.trim()) {
            return Err(format!("items[{idx}].status is invalid: {}", item.status));
        }

        if item.reason.trim().is_empty() {
            return Err(format!("items[{idx}].reason must not be empty"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(status: &str) -> LocalMappingPreflightItem {
        LocalMappingPreflightItem {
            status: status.to_string(),
            reason: "ok".to_string(),
            ..LocalMappingPreflightItem::default()
        }
    }

    fn base_report() -> LocalMappingPreflightReport {
        LocalMappingPreflightReport {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            report_id: "report-1".to_string(),
            reported_at_utc: "2026-05-20T12:00:00Z".to_string(),
            source: Some("client".to_string()),
            status: STATUS_WARNING.to_string(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            summary: LocalMappingPreflightSummary::default(),
            items: vec![],
            metadata: None,
        }
    }

    #[test]
    fn compute_summary_counts_already_mapped() {
        let summary = compute_local_mapping_preflight_summary(&[item(ITEM_STATUS_ALREADY_MAPPED)]);
        assert_eq!(summary.total_items, 1);
        assert_eq!(summary.applicable_items, 1);
        assert_eq!(summary.already_mapped, 1);
    }

    #[test]
    fn compute_summary_counts_missing_mapping() {
        let summary = compute_local_mapping_preflight_summary(&[item(ITEM_STATUS_MISSING_MAPPING)]);
        assert_eq!(summary.missing_mapping, 1);
        assert_eq!(summary.applicable_items, 1);
    }

    #[test]
    fn compute_summary_counts_ambiguous() {
        let summary = compute_local_mapping_preflight_summary(&[item(ITEM_STATUS_AMBIGUOUS)]);
        assert_eq!(summary.ambiguous, 1);
        assert_eq!(summary.applicable_items, 1);
    }

    #[test]
    fn compute_summary_counts_not_applicable() {
        let summary = compute_local_mapping_preflight_summary(&[item(ITEM_STATUS_NOT_APPLICABLE)]);
        assert_eq!(summary.not_applicable, 1);
        assert_eq!(summary.applicable_items, 0);
    }

    #[test]
    fn validation_accepts_empty_report_with_status_empty() {
        let mut report = base_report();
        report.status = STATUS_EMPTY.to_string();
        assert!(validate_local_mapping_preflight_report(&report).is_ok());
    }

    #[test]
    fn validation_rejects_invalid_report_status() {
        let mut report = base_report();
        report.status = "explode".to_string();
        assert!(validate_local_mapping_preflight_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_invalid_item_status() {
        let mut report = base_report();
        report.items = vec![item("explode")];
        assert!(validate_local_mapping_preflight_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_safe_to_auto_execute_true() {
        let mut report = base_report();
        report.safe_to_auto_execute = true;
        assert!(validate_local_mapping_preflight_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_side_effects_performed_true() {
        let mut report = base_report();
        report.side_effects_performed = true;
        assert!(validate_local_mapping_preflight_report(&report).is_err());
    }

    #[test]
    fn normalization_overrides_project_id_and_node_id() {
        let normalized = normalize_local_mapping_preflight_report(
            "project-a",
            "node-b",
            LocalMappingPreflightReport {
                project_id: "wrong".to_string(),
                node_id: "wrong".to_string(),
                status: STATUS_WARNING.to_string(),
                safe_to_auto_execute: false,
                side_effects_performed: false,
                ..LocalMappingPreflightReport::default()
            },
        );

        assert_eq!(normalized.project_id, "project-a");
        assert_eq!(normalized.node_id, "node-b");
        assert!(!normalized.report_id.is_empty());
        assert!(!normalized.reported_at_utc.is_empty());
    }

    #[test]
    fn normalization_defaults_source_to_client() {
        let normalized = normalize_local_mapping_preflight_report(
            "project-a",
            "node-b",
            LocalMappingPreflightReport {
                status: STATUS_WARNING.to_string(),
                safe_to_auto_execute: false,
                side_effects_performed: false,
                ..LocalMappingPreflightReport::default()
            },
        );

        assert_eq!(normalized.source.as_deref(), Some("client"));
    }

    #[test]
    fn normalization_recomputes_summary() {
        let normalized = normalize_local_mapping_preflight_report(
            "project-a",
            "node-b",
            LocalMappingPreflightReport {
                status: STATUS_WARNING.to_string(),
                safe_to_auto_execute: false,
                side_effects_performed: false,
                summary: LocalMappingPreflightSummary {
                    total_items: 99,
                    applicable_items: 99,
                    already_mapped: 99,
                    missing_mapping: 99,
                    ambiguous: 99,
                    not_applicable: 99,
                    warnings: 99,
                },
                items: vec![LocalMappingPreflightItem {
                    status: ITEM_STATUS_MISSING_MAPPING.to_string(),
                    reason: "no trusted local mapping found".to_string(),
                    source_task: Some(json!({ "kept": true })),
                    ..LocalMappingPreflightItem::default()
                }],
                ..LocalMappingPreflightReport::default()
            },
        );

        assert_eq!(
            normalized.summary,
            LocalMappingPreflightSummary {
                total_items: 1,
                applicable_items: 1,
                already_mapped: 0,
                missing_mapping: 1,
                ambiguous: 0,
                not_applicable: 0,
                warnings: 0,
            }
        );
    }
}
