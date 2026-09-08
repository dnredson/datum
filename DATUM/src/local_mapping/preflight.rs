use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    config::Config, dispatch::container_executor::find_trusted_container_mapping_matches,
    installation_tasks::model::InstallationTask,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalMappingPreflightReport {
    #[serde(default)]
    pub project_id: Option<String>,
    pub node_id: String,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub reported_at_utc: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    pub status: String,
    #[serde(default)]
    pub items: Vec<LocalMappingPreflightItem>,
    pub summary: LocalMappingPreflightSummary,
    pub safe_to_auto_execute: bool,
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
    pub status: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalMappingPreflightSummary {
    pub total_items: u64,
    pub applicable_items: u64,
    pub already_mapped: u64,
    pub missing_mapping: u64,
    pub ambiguous: u64,
    pub not_applicable: u64,
    pub warnings: u64,
}

pub fn run_local_mapping_preflight(
    config: &Config,
    node_id: &str,
    tasks: &[InstallationTask],
) -> LocalMappingPreflightReport {
    let items: Vec<LocalMappingPreflightItem> = tasks
        .iter()
        .map(|task| build_preflight_item(config, node_id, task))
        .collect();
    let summary = summarize_items(&items);
    let status = derive_report_status(&summary);

    LocalMappingPreflightReport {
        project_id: None,
        node_id: node_id.to_string(),
        report_id: None,
        reported_at_utc: None,
        source: Some("client".to_string()),
        status,
        items,
        summary,
        safe_to_auto_execute: false,
        side_effects_performed: false,
        metadata: None,
    }
}

fn build_preflight_item(
    config: &Config,
    node_id: &str,
    task: &InstallationTask,
) -> LocalMappingPreflightItem {
    let applicable = is_local_mapping_applicable(task);
    let source_task = serde_json::to_value(task).ok();

    if !applicable {
        return LocalMappingPreflightItem {
            task_id: task.task_id.clone(),
            installation_id: task.installation_id.clone(),
            planning_id: task.planning_id.clone(),
            action_id: task.action_id.clone(),
            module_id: task.module_id.clone(),
            runtime: task.runtime.clone(),
            artifact: task.artifact.clone(),
            required_step: task.required_step.clone(),
            installation_action: task.installation_action.clone(),
            target_node: task.node_id.clone().or_else(|| Some(node_id.to_string())),
            status: "not_applicable".to_string(),
            reason: "task does not require local mapping preflight".to_string(),
            current_mapping_source: None,
            current_container_name: None,
            suggested_mapping_key: None,
            suggested_artifact: None,
            source_task,
        };
    }

    let candidates = task_mapping_candidates(task);
    let matches = find_trusted_container_mapping_matches(&candidates, config);
    let unique_containers: BTreeSet<String> = matches
        .iter()
        .map(|entry| entry.container_name.to_ascii_lowercase())
        .collect();

    if unique_containers.len() == 1 {
        let container_name = matches
            .first()
            .map(|entry| entry.container_name.clone())
            .unwrap_or_default();
        let sources = distinct_sources(&matches);

        return LocalMappingPreflightItem {
            task_id: task.task_id.clone(),
            installation_id: task.installation_id.clone(),
            planning_id: task.planning_id.clone(),
            action_id: task.action_id.clone(),
            module_id: task.module_id.clone(),
            runtime: task.runtime.clone(),
            artifact: task.artifact.clone(),
            required_step: task.required_step.clone(),
            installation_action: task.installation_action.clone(),
            target_node: task.node_id.clone().or_else(|| Some(node_id.to_string())),
            status: "already_mapped".to_string(),
            reason: "trusted local mapping exists".to_string(),
            current_mapping_source: Some(sources.join("+")),
            current_container_name: Some(container_name),
            suggested_mapping_key: None,
            suggested_artifact: None,
            source_task,
        };
    }

    if unique_containers.len() > 1 {
        let options = matches
            .iter()
            .map(|entry| format!("{}:{}", entry.source, entry.container_name))
            .collect::<Vec<_>>()
            .join(", ");

        return LocalMappingPreflightItem {
            task_id: task.task_id.clone(),
            installation_id: task.installation_id.clone(),
            planning_id: task.planning_id.clone(),
            action_id: task.action_id.clone(),
            module_id: task.module_id.clone(),
            runtime: task.runtime.clone(),
            artifact: task.artifact.clone(),
            required_step: task.required_step.clone(),
            installation_action: task.installation_action.clone(),
            target_node: task.node_id.clone().or_else(|| Some(node_id.to_string())),
            status: "ambiguous".to_string(),
            reason: format!("multiple trusted local mappings match this task: {options}"),
            current_mapping_source: Some(distinct_sources(&matches).join("+")),
            current_container_name: None,
            suggested_mapping_key: preferred_mapping_key(task),
            suggested_artifact: task.artifact.clone(),
            source_task,
        };
    }

    LocalMappingPreflightItem {
        task_id: task.task_id.clone(),
        installation_id: task.installation_id.clone(),
        planning_id: task.planning_id.clone(),
        action_id: task.action_id.clone(),
        module_id: task.module_id.clone(),
        runtime: task.runtime.clone(),
        artifact: task.artifact.clone(),
        required_step: task.required_step.clone(),
        installation_action: task.installation_action.clone(),
        target_node: task.node_id.clone().or_else(|| Some(node_id.to_string())),
        status: "missing_mapping".to_string(),
        reason: "no trusted local mapping found in DATUM config".to_string(),
        current_mapping_source: None,
        current_container_name: None,
        suggested_mapping_key: preferred_mapping_key(task),
        suggested_artifact: task.artifact.clone(),
        source_task,
    }
}

fn summarize_items(items: &[LocalMappingPreflightItem]) -> LocalMappingPreflightSummary {
    let mut summary = LocalMappingPreflightSummary {
        total_items: items.len() as u64,
        ..LocalMappingPreflightSummary::default()
    };

    for item in items {
        match item.status.as_str() {
            "already_mapped" => {
                summary.applicable_items += 1;
                summary.already_mapped += 1;
            }
            "missing_mapping" => {
                summary.applicable_items += 1;
                summary.missing_mapping += 1;
                summary.warnings += 1;
            }
            "ambiguous" => {
                summary.applicable_items += 1;
                summary.ambiguous += 1;
                summary.warnings += 1;
            }
            "warning" => {
                summary.applicable_items += 1;
                summary.warnings += 1;
            }
            _ => {
                summary.not_applicable += 1;
            }
        }
    }

    summary
}

fn derive_report_status(summary: &LocalMappingPreflightSummary) -> String {
    if summary.applicable_items == 0 {
        return "empty".to_string();
    }

    if summary.ambiguous > 0 {
        return "failed".to_string();
    }

    if summary.missing_mapping > 0 || summary.warnings > 0 {
        return "warning".to_string();
    }

    "passed".to_string()
}

fn is_local_mapping_applicable(task: &InstallationTask) -> bool {
    equals_ignore_ascii(task.required_step.as_deref(), "add_local_mapping")
        || equals_ignore_ascii(task.installation_action.as_deref(), "prepare_local_mapping")
        || contains_local_mapping_phrase(task.reason.as_deref())
        || value_indicates_local_mapping(task.source_installation_item.as_ref())
        || value_indicates_local_mapping(task.metadata.as_ref())
        || task
            .extra
            .get("source_planning_item")
            .map(|value| value_indicates_local_mapping(Some(value)))
            .unwrap_or(false)
        || task
            .extra
            .get("source_task")
            .map(|value| value_indicates_local_mapping(Some(value)))
            .unwrap_or(false)
}

fn preferred_mapping_key(task: &InstallationTask) -> Option<String> {
    task.module_id
        .clone()
        .or_else(|| task.artifact.clone())
        .or_else(|| task.action_id.clone())
}

fn task_mapping_candidates(task: &InstallationTask) -> Vec<String> {
    let mut out = BTreeSet::new();

    for value in [
        task.module_id.as_deref(),
        task.artifact.as_deref(),
        task.action_id.as_deref(),
        task.installation_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let normalized = value.trim();
        if !normalized.is_empty() {
            out.insert(normalized.to_string());
        }
    }

    for value in [
        task.source_installation_item.as_ref(),
        task.metadata.as_ref(),
        task.extra.get("source_planning_item"),
        task.extra.get("source_task"),
    ]
    .into_iter()
    .flatten()
    {
        collect_string_candidates(value, &mut out);
    }

    out.into_iter().collect()
}

fn collect_string_candidates(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for key in [
                "module_id",
                "artifact",
                "action_id",
                "service_name",
                "container_name",
            ] {
                if let Some(candidate) = map.get(key).and_then(|inner| inner.as_str()) {
                    let normalized = candidate.trim();
                    if !normalized.is_empty() {
                        out.insert(normalized.to_string());
                    }
                }
            }

            for nested in map.values() {
                collect_string_candidates(nested, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_string_candidates(item, out);
            }
        }
        _ => {}
    }
}

fn value_indicates_local_mapping(value: Option<&Value>) -> bool {
    value
        .map(value_contains_local_mapping_marker)
        .unwrap_or(false)
}

fn value_contains_local_mapping_marker(value: &Value) -> bool {
    match value {
        Value::String(text) => contains_local_mapping_phrase(Some(text)),
        Value::Array(items) => items.iter().any(value_contains_local_mapping_marker),
        Value::Object(map) => map.values().any(value_contains_local_mapping_marker),
        _ => false,
    }
}

fn contains_local_mapping_phrase(value: Option<&str>) -> bool {
    value
        .map(|text| text.to_ascii_lowercase())
        .map(|text| {
            text.contains("local mapping")
                || text.contains("trusted local mapping")
                || text.contains("missing_local_mapping")
        })
        .unwrap_or(false)
}

fn equals_ignore_ascii(value: Option<&str>, expected: &str) -> bool {
    value
        .map(|current| current.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn distinct_sources(
    matches: &[crate::dispatch::container_executor::TrustedContainerMappingMatch],
) -> Vec<String> {
    let mut sources = BTreeSet::new();
    for entry in matches {
        sources.insert(entry.source.to_string());
    }
    sources.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{
        is_local_mapping_applicable, run_local_mapping_preflight, LocalMappingPreflightReport,
    };
    use crate::{
        config::{
            AgentConfig, Config, DispatchConfig, DispatchContainerMapping, OutputsConfig,
            ServiceRegistryConfig, ServiceRegistryService, ServiceSelector,
        },
        installation_tasks::model::InstallationTask,
    };

    fn base_config() -> Config {
        Config {
            agent: AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "0.0.1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: OutputsConfig {
                file_json: None,
                http: None,
            },
            dispatch: Some(DispatchConfig {
                enabled: false,
                url: None,
                project_id: None,
                timeout_s: None,
                execute: None,
                publish_results: None,
                allow_container_execution: Some(false),
                container_mappings: Vec::new(),
            }),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    fn mapping_task() -> InstallationTask {
        InstallationTask {
            task_id: Some("task-1".to_string()),
            installation_id: Some("installation-1".to_string()),
            planning_id: Some("planning-1".to_string()),
            action_id: Some("action-1".to_string()),
            module_id: Some("chirpstack-primary".to_string()),
            runtime: Some("container".to_string()),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
            required_step: Some("add_local_mapping".to_string()),
            installation_action: Some("prepare_local_mapping".to_string()),
            reason: Some("missing trusted local mapping for node".to_string()),
            node_id: Some("fog-01".to_string()),
            ..InstallationTask::default()
        }
    }

    fn assert_invariants(report: &LocalMappingPreflightReport) {
        assert!(!report.safe_to_auto_execute);
        assert!(!report.side_effects_performed);
    }

    #[test]
    fn applicable_task_with_required_step_is_detected() {
        assert!(is_local_mapping_applicable(&InstallationTask {
            required_step: Some("add_local_mapping".to_string()),
            ..InstallationTask::default()
        }));
    }

    #[test]
    fn applicable_task_with_installation_action_is_detected() {
        assert!(is_local_mapping_applicable(&InstallationTask {
            installation_action: Some("prepare_local_mapping".to_string()),
            ..InstallationTask::default()
        }));
    }

    #[test]
    fn non_applicable_task_is_reported_as_not_applicable() {
        let report = run_local_mapping_preflight(
            &base_config(),
            "fog-01",
            &[InstallationTask {
                task_id: Some("task-1".to_string()),
                installation_action: Some("enable_runtime".to_string()),
                ..InstallationTask::default()
            }],
        );

        assert_eq!(report.status, "empty");
        assert_eq!(report.items[0].status, "not_applicable");
        assert_invariants(&report);
    }

    #[test]
    fn existing_container_mapping_produces_already_mapped() {
        let mut cfg = base_config();
        cfg.dispatch.as_mut().expect("dispatch").container_mappings =
            vec![DispatchContainerMapping {
                module_id: Some("chirpstack-primary".to_string()),
                service_name: None,
                container_name: "chirpstack".to_string(),
                artifact: Some("chirpstack/chirpstack:latest".to_string()),
                ports: Vec::new(),
            }];

        let report = run_local_mapping_preflight(&cfg, "fog-01", &[mapping_task()]);

        assert_eq!(report.status, "passed");
        assert_eq!(report.items[0].status, "already_mapped");
        assert_eq!(
            report.items[0].current_mapping_source.as_deref(),
            Some("container_mappings")
        );
        assert_eq!(
            report.items[0].current_container_name.as_deref(),
            Some("chirpstack")
        );
        assert_invariants(&report);
    }

    #[test]
    fn existing_service_registry_mapping_produces_already_mapped() {
        let mut cfg = base_config();
        cfg.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![ServiceRegistryService {
                name: "chirpstack-primary".to_string(),
                runtime: "container".to_string(),
                selector: Some(ServiceSelector {
                    container_name: Some("chirpstack".to_string()),
                    process_name: None,
                }),
                ..ServiceRegistryService::default()
            }],
        });

        let report = run_local_mapping_preflight(&cfg, "fog-01", &[mapping_task()]);

        assert_eq!(report.status, "passed");
        assert_eq!(report.items[0].status, "already_mapped");
        assert_eq!(
            report.items[0].current_mapping_source.as_deref(),
            Some("service_registry")
        );
        assert_invariants(&report);
    }

    #[test]
    fn no_mapping_produces_missing_mapping() {
        let report = run_local_mapping_preflight(&base_config(), "fog-01", &[mapping_task()]);

        assert_eq!(report.status, "warning");
        assert_eq!(report.items[0].status, "missing_mapping");
        assert_eq!(
            report.items[0].suggested_mapping_key.as_deref(),
            Some("chirpstack-primary")
        );
        assert_invariants(&report);
    }

    #[test]
    fn conflicting_mappings_produce_ambiguous() {
        let mut cfg = base_config();
        cfg.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![ServiceRegistryService {
                name: "chirpstack-primary".to_string(),
                runtime: "container".to_string(),
                selector: Some(ServiceSelector {
                    container_name: Some("chirpstack-a".to_string()),
                    process_name: None,
                }),
                ..ServiceRegistryService::default()
            }],
        });
        cfg.dispatch.as_mut().expect("dispatch").container_mappings =
            vec![DispatchContainerMapping {
                module_id: Some("chirpstack-primary".to_string()),
                service_name: None,
                container_name: "chirpstack-b".to_string(),
                artifact: Some("chirpstack/chirpstack:latest".to_string()),
                ports: Vec::new(),
            }];

        let report = run_local_mapping_preflight(&cfg, "fog-01", &[mapping_task()]);

        assert_eq!(report.status, "failed");
        assert_eq!(report.items[0].status, "ambiguous");
        assert!(report.items[0]
            .reason
            .contains("multiple trusted local mappings"));
        assert_invariants(&report);
    }

    #[test]
    fn report_summary_counts_are_correct() {
        let mut cfg = base_config();
        cfg.dispatch.as_mut().expect("dispatch").container_mappings =
            vec![DispatchContainerMapping {
                module_id: Some("chirpstack-primary".to_string()),
                service_name: None,
                container_name: "chirpstack".to_string(),
                artifact: Some("chirpstack/chirpstack:latest".to_string()),
                ports: Vec::new(),
            }];

        let report = run_local_mapping_preflight(
            &cfg,
            "fog-01",
            &[
                mapping_task(),
                InstallationTask {
                    task_id: Some("task-2".to_string()),
                    installation_action: Some("enable_runtime".to_string()),
                    ..InstallationTask::default()
                },
                InstallationTask {
                    task_id: Some("task-3".to_string()),
                    required_step: Some("add_local_mapping".to_string()),
                    module_id: Some("missing".to_string()),
                    ..InstallationTask::default()
                },
            ],
        );

        assert_eq!(report.summary.total_items, 3);
        assert_eq!(report.summary.applicable_items, 2);
        assert_eq!(report.summary.already_mapped, 1);
        assert_eq!(report.summary.missing_mapping, 1);
        assert_eq!(report.summary.not_applicable, 1);
        assert_eq!(report.summary.warnings, 1);
        assert_invariants(&report);
    }

    #[test]
    fn report_serialization_includes_status_items_and_summary() {
        let report = run_local_mapping_preflight(&base_config(), "fog-01", &[mapping_task()]);
        let value = serde_json::to_value(&report).expect("report should serialize");

        assert_eq!(value["status"].as_str(), Some("warning"));
        assert!(value["items"].is_array());
        assert!(value["summary"].is_object());
        assert_eq!(value["safe_to_auto_execute"].as_bool(), Some(false));
        assert_eq!(value["side_effects_performed"].as_bool(), Some(false));
    }

    #[test]
    fn source_guard_confirms_no_docker_shell_or_config_writes() {
        let source = include_str!("preflight.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "write(",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "local mapping preflight must not contain forbidden token: {forbidden}"
            );
        }
    }
}
