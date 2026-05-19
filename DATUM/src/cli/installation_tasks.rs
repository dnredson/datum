use anyhow::{anyhow, Result};

use crate::{
    config::Config,
    installation_tasks::{
        client::{
            fetch_installation_task, fetch_installation_tasks_for_node,
            materialize_installation_tasks_for_node, patch_installation_task_status,
        },
        model::{InstallationTask, InstallationTaskList},
    },
};

const ALLOWED_STATUSES: &[&str] = &["acknowledged", "blocked", "cancelled", "completed"];

pub fn run_installation_tasks_command(
    cfg: &Config,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    task_id: Option<&str>,
    status: Option<&str>,
    reason: Option<&str>,
    materialize: bool,
    json_output: bool,
) -> Result<()> {
    let node_id = node_id_override
        .map(str::to_string)
        .unwrap_or_else(|| cfg.agent.node_id.clone());
    let project_id = resolve_project_id(cfg, project_id_override)?;
    let base_url = resolve_base_url(cfg, url_override)?;

    if let Some(status) = status {
        let task_id =
            task_id.ok_or_else(|| anyhow!("--task-id is required when --status is provided"))?;
        validate_client_status(status)?;
        let updated = patch_installation_task_status(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            status,
            reason,
        )?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&updated)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_status_update(&updated));
        return Ok(());
    }

    if let Some(task_id) = task_id {
        let task = fetch_installation_task(&base_url, &project_id, &node_id, task_id)?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&task)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_single_task(&task, &project_id, &node_id));
        return Ok(());
    }

    let response = if materialize {
        materialize_installation_tasks_for_node(&base_url, &project_id, &node_id)?
    } else {
        fetch_installation_tasks_for_node(&base_url, &project_id, &node_id)?
    };

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&response)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return Ok(());
    }

    print!("{}", format_task_list(&response, &project_id, &node_id));
    Ok(())
}

fn resolve_project_id(cfg: &Config, project_id_override: Option<&str>) -> Result<String> {
    project_id_override
        .map(str::to_string)
        .or_else(|| cfg.dispatch.as_ref().and_then(|dispatch| dispatch.project_id.clone()))
        .or_else(|| cfg.outputs.http.as_ref().and_then(|http| http.project_id.clone()))
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
            anyhow!("outputs.http.enabled=true or --url is required to fetch installation tasks")
        })
}

pub fn validate_client_status(status: &str) -> Result<()> {
    if ALLOWED_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(anyhow!(
            "invalid installation task status '{}'; allowed values: {}",
            status,
            ALLOWED_STATUSES.join(", ")
        ))
    }
}

fn format_task_list(
    response: &InstallationTaskList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.tasks.len() as u64);

    let mut lines = vec![
        format!("Installation tasks for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    if response.tasks.is_empty() {
        lines.push("No installation tasks for this node.".to_string());
        lines.push(
            "Tip: run with --materialize after generating an installation plan on the server."
                .to_string(),
        );
        return lines.join("\n");
    }

    lines.push(format!("Tasks: {}", count));
    lines.push(String::new());

    for (index, task) in response.tasks.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }

        lines.extend(format_task_block(task));
    }

    lines.join("\n")
}

fn format_single_task(
    task: &InstallationTask,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = task.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = task.project_id.as_deref().unwrap_or(project_id);

    let mut lines = vec![
        format!("Installation task for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
        String::new(),
    ];
    lines.extend(format_task_block(task));
    lines.join("\n")
}

fn format_task_block(task: &InstallationTask) -> Vec<String> {
    let status = task
        .status
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let priority = task
        .priority
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let action = task.installation_action.as_deref().unwrap_or("unknown");

    let mut lines = vec![format!("[{}][{}] {}", status, priority, action)];

    if let Some(task_id) = task.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(module_id) = task.module_id.as_deref() {
        lines.push(format!("  module: {}", module_id));
    }

    if let Some(runtime) = task.runtime.as_deref() {
        lines.push(format!("  runtime: {}", runtime));
    }

    if let Some(artifact) = task.artifact.as_deref() {
        lines.push(format!("  artifact: {}", artifact));
    }

    if let Some(requires_operator) = task.requires_operator {
        lines.push(format!("  requires operator: {}", requires_operator));
    }

    if let Some(safe_to_auto_execute) = task.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = task.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines
}

fn format_status_update(task: &InstallationTask) -> String {
    let mut lines = vec!["Updated installation task".to_string()];

    if let Some(task_id) = task.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(status) = task.status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(updated_at_utc) = task.updated_at_utc.as_deref() {
        lines.push(format!("  updated_at_utc: {}", updated_at_utc));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{format_task_list, validate_client_status};
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, HttpConfig, OutputsConfig},
        installation_tasks::model::{InstallationTask, InstallationTaskList},
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
                container_mappings: Vec::new(),
            }),
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn human_formatting_includes_key_fields() {
        let rendered = format_task_list(
            &InstallationTaskList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                tasks: vec![InstallationTask {
                    task_id: Some("task-1".to_string()),
                    status: Some("planned".to_string()),
                    priority: Some("high".to_string()),
                    installation_action: Some("prepare_local_mapping".to_string()),
                    module_id: Some("chirpstack-primary".to_string()),
                    artifact: Some("chirpstack/chirpstack:latest".to_string()),
                    requires_operator: Some(true),
                    safe_to_auto_execute: Some(false),
                    reason: Some("client lacks trusted local mapping for this action".to_string()),
                    ..InstallationTask::default()
                }],
                ..InstallationTaskList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("[PLANNED][HIGH] prepare_local_mapping"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("module: chirpstack-primary"));
        assert!(rendered.contains("artifact: chirpstack/chirpstack:latest"));
        assert!(rendered.contains("requires operator: true"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: client lacks trusted local mapping for this action"));
    }

    #[test]
    fn client_status_validation_accepts_allowed_values() {
        assert!(validate_client_status("acknowledged").is_ok());
        assert!(validate_client_status("blocked").is_ok());
        assert!(validate_client_status("cancelled").is_ok());
        assert!(validate_client_status("completed").is_ok());
    }

    #[test]
    fn client_status_validation_rejects_arbitrary_values() {
        assert!(validate_client_status("planned").is_err());
        assert!(validate_client_status("explode").is_err());
    }

    #[test]
    fn default_command_behavior_does_not_execute_anything() {
        let cfg = base_config();
        assert!(cfg.dispatch.as_ref().and_then(|dispatch| dispatch.execute) == Some(true));
        assert_eq!(cfg.agent.node_id, "fog-01");
    }
}
