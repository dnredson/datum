use anyhow::{anyhow, Result};

use crate::{
    config::Config,
    installation_plan::{
        client::fetch_installation_plan_for_node,
        model::{InstallationPlanItem, InstallationPlanResponse},
    },
};

pub fn run_installation_plan_command(
    cfg: &Config,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let node_id = node_id_override
        .map(str::to_string)
        .unwrap_or_else(|| cfg.agent.node_id.clone());
    let project_id = resolve_project_id(cfg, project_id_override)?;
    let base_url = resolve_base_url(cfg, url_override)?;

    let response = fetch_installation_plan_for_node(&base_url, &project_id, &node_id)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return Ok(());
    }

    print!(
        "{}",
        format_installation_plan(&response, &project_id, &node_id)
    );
    Ok(())
}

fn resolve_project_id(cfg: &Config, project_id_override: Option<&str>) -> Result<String> {
    project_id_override
        .map(str::to_string)
        .or_else(|| {
            cfg.dispatch
                .as_ref()
                .and_then(|dispatch| dispatch.project_id.clone())
        })
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .and_then(|http| http.project_id.clone())
        })
        .ok_or_else(|| {
            anyhow!("project_id is required in --project-id, dispatch.project_id, or outputs.http.project_id")
        })
}

fn resolve_base_url(cfg: &Config, url_override: Option<&str>) -> Result<String> {
    url_override
        .map(str::to_string)
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .filter(|http| http.enabled)
                .map(|http| http.url.clone())
        })
        .ok_or_else(|| {
            anyhow!("outputs.http.enabled=true or --url is required to fetch installation plan")
        })
}

fn format_installation_plan(
    response: &InstallationPlanResponse,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);

    let mut lines = vec![
        format!("Installation plan for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    let items = &response.installation_plan.items;
    if items.is_empty() {
        lines.push("No installation plan items for this node.".to_string());
        return lines.join("\n");
    }

    let summary = response.installation_plan.summary.as_ref();
    lines.push(format!(
        "Items: {}",
        summary
            .and_then(|s| s.total_items)
            .unwrap_or(items.len() as u64)
    ));
    lines.push(format!(
        "Operator required: {} | Auto-executable: {}",
        summary
            .and_then(|s| s.operator_required)
            .unwrap_or(items.len() as u64),
        summary.and_then(|s| s.auto_executable).unwrap_or(0)
    ));
    lines.push(format!(
        "High: {} | Medium: {} | Low: {}",
        summary
            .and_then(|s| s.high)
            .unwrap_or_else(|| count_priority(items, "high")),
        summary
            .and_then(|s| s.medium)
            .unwrap_or_else(|| count_priority(items, "medium")),
        summary
            .and_then(|s| s.low)
            .unwrap_or_else(|| count_priority(items, "low"))
    ));
    lines.push(String::new());

    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }

        let priority = item
            .priority
            .as_deref()
            .unwrap_or("unknown")
            .to_ascii_uppercase();
        let installation_action = item.installation_action.as_deref().unwrap_or("unknown");

        lines.push(format!("[{}] {}", priority, installation_action));

        if let Some(module_id) = item.module_id.as_deref() {
            lines.push(format!("  module: {}", module_id));
        }

        if let Some(runtime) = item.runtime.as_deref() {
            lines.push(format!("  runtime: {}", runtime));
        }

        if let Some(artifact) = item.artifact.as_deref() {
            lines.push(format!("  artifact: {}", artifact));
        }

        if let Some(status) = item.status.as_deref() {
            lines.push(format!("  status: {}", status));
        }

        if let Some(requires_operator) = item.requires_operator {
            lines.push(format!("  requires operator: {}", requires_operator));
        }

        if let Some(safe_to_auto_execute) = item.safe_to_auto_execute {
            lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
        }

        if let Some(reason) = item.reason.as_deref() {
            lines.push(format!("  reason: {}", reason));
        }
    }

    lines.join("\n")
}

fn count_priority(items: &[InstallationPlanItem], expected: &str) -> u64 {
    items
        .iter()
        .filter(|item| item.priority.as_deref() == Some(expected))
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::{format_installation_plan, resolve_base_url, resolve_project_id};
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, HttpConfig, OutputsConfig},
        installation_plan::model::{
            InstallationPlan, InstallationPlanItem, InstallationPlanResponse,
            InstallationPlanSummary,
        },
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
                http: Some(HttpConfig {
                    enabled: true,
                    url: "http://localhost:8080/api/v1/snapshots".to_string(),
                    project_id: Some("smart-city-campinas".to_string()),
                    timeout_s: Some(10),
                }),
            },
            dispatch: Some(DispatchConfig {
                enabled: true,
                url: None,
                project_id: Some("smart-city-campinas".to_string()),
                timeout_s: None,
                execute: Some(true),
                publish_results: Some(true),
                allow_container_execution: Some(true),
                configuration_root: None,
                secret_bindings: Vec::new(),
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

    #[test]
    fn human_formatting_includes_key_fields() {
        let response = InstallationPlanResponse {
            project_id: Some("smart-city-campinas".to_string()),
            node_id: Some("fog-01".to_string()),
            installation_plan: InstallationPlan {
                installer_version: Some("0.1.0".to_string()),
                source: Some("deployment_planning".to_string()),
                items: vec![InstallationPlanItem {
                    installation_id: Some("installation-1".to_string()),
                    module_id: Some("chirpstack-primary".to_string()),
                    runtime: Some("container".to_string()),
                    artifact: Some("chirpstack/chirpstack:latest".to_string()),
                    installation_action: Some("prepare_local_mapping".to_string()),
                    status: Some("planned".to_string()),
                    priority: Some("high".to_string()),
                    reason: Some("client lacks trusted local mapping for this action".to_string()),
                    requires_operator: Some(true),
                    safe_to_auto_execute: Some(false),
                    ..InstallationPlanItem::default()
                }],
                summary: Some(InstallationPlanSummary {
                    total_items: Some(1),
                    operator_required: Some(1),
                    auto_executable: Some(0),
                    high: Some(1),
                    medium: Some(0),
                    low: Some(0),
                    ..InstallationPlanSummary::default()
                }),
                ..InstallationPlan::default()
            },
            ..InstallationPlanResponse::default()
        };

        let rendered = format_installation_plan(&response, "smart-city-campinas", "fog-01");

        assert!(rendered.contains("[HIGH] prepare_local_mapping"));
        assert!(rendered.contains("module: chirpstack-primary"));
        assert!(rendered.contains("artifact: chirpstack/chirpstack:latest"));
        assert!(rendered.contains("requires operator: true"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: client lacks trusted local mapping for this action"));
    }

    #[test]
    fn human_formatting_handles_missing_optional_fields() {
        let response = InstallationPlanResponse {
            installation_plan: InstallationPlan {
                items: vec![InstallationPlanItem {
                    priority: Some("medium".to_string()),
                    installation_action: Some("investigate_previous_failure".to_string()),
                    ..InstallationPlanItem::default()
                }],
                summary: None,
                ..InstallationPlan::default()
            },
            ..InstallationPlanResponse::default()
        };

        let rendered = format_installation_plan(&response, "smart-city-campinas", "fog-01");

        assert!(rendered.contains("Installation plan for node fog-01"));
        assert!(rendered.contains("[MEDIUM] investigate_previous_failure"));
        assert!(!rendered.contains("module:"));
        assert!(!rendered.contains("artifact:"));
        assert!(!rendered.contains("requires operator:"));
    }

    #[test]
    fn human_formatting_handles_empty_items() {
        let response = InstallationPlanResponse {
            installation_plan: InstallationPlan::default(),
            ..InstallationPlanResponse::default()
        };

        let rendered = format_installation_plan(&response, "smart-city-campinas", "fog-01");

        assert!(rendered.contains("No installation plan items for this node."));
    }

    #[test]
    fn resolve_project_id_prefers_override() {
        let cfg = base_config();

        let project_id =
            resolve_project_id(&cfg, Some("override-project")).expect("project id should resolve");

        assert_eq!(project_id, "override-project");
    }

    #[test]
    fn resolve_base_url_uses_output_http() {
        let cfg = base_config();

        let url = resolve_base_url(&cfg, None).expect("base URL should resolve");

        assert_eq!(url, "http://localhost:8080/api/v1/snapshots");
    }
}
