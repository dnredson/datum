use anyhow::{anyhow, Result};

use crate::{
    config::Config,
    installation_tasks::{
        client::{
            fetch_installation_task, fetch_installation_task_approvals_for_node,
            fetch_installation_task_readiness, fetch_installation_task_readiness_for_node,
            fetch_installation_tasks_for_node, fetch_latest_installation_task_approval,
            materialize_installation_tasks_for_node, patch_installation_task_status,
            submit_installation_task_approval,
        },
        model::{
            InstallationTask, InstallationTaskApproval, InstallationTaskApprovalList,
            InstallationTaskList, InstallationTaskReadiness, InstallationTaskReadinessList,
        },
    },
};

const ALLOWED_STATUSES: &[&str] = &["acknowledged", "blocked", "cancelled", "completed"];

pub fn run_installation_tasks_command(
    cfg: &Config,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    task_id: Option<&str>,
    readiness: bool,
    approvals: bool,
    approval: bool,
    approve: bool,
    approved_by: Option<&str>,
    status: Option<&str>,
    reason: Option<&str>,
    expires_at_utc: Option<&str>,
    materialize: bool,
    json_output: bool,
) -> Result<()> {
    let node_id = node_id_override
        .map(str::to_string)
        .unwrap_or_else(|| cfg.agent.node_id.clone());
    let project_id = resolve_project_id(cfg, project_id_override)?;
    let base_url = resolve_base_url(cfg, url_override)?;
    validate_command_mode(
        task_id,
        readiness,
        approvals,
        approval,
        approve,
        status,
        materialize,
        approved_by,
        expires_at_utc,
    )?;

    if readiness {
        if let Some(task_id) = task_id {
            match fetch_installation_task_readiness(&base_url, &project_id, &node_id, task_id) {
                Ok(response) => {
                    if json_output {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&response).unwrap_or_else(|_| {
                                "{\"error\":\"failed to render JSON\"}".to_string()
                            })
                        );
                        return Ok(());
                    }

                    print!("{}", format_single_readiness(&response, task_id));
                    return Ok(());
                }
                Err(error) if error.to_string().contains("status=404") => {
                    if json_output {
                        return Err(error);
                    }

                    println!("No readiness record found for task {}.", task_id);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }

        let response =
            fetch_installation_task_readiness_for_node(&base_url, &project_id, &node_id)?;

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
            format_readiness_list(&response, &project_id, &node_id)
        );
        return Ok(());
    }

    if approvals {
        let response =
            fetch_installation_task_approvals_for_node(&base_url, &project_id, &node_id)?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_approval_list(&response, &project_id, &node_id));
        return Ok(());
    }

    if approval {
        let task_id =
            task_id.ok_or_else(|| anyhow!("--task-id is required when --approval is provided"))?;
        match fetch_latest_installation_task_approval(&base_url, &project_id, &node_id, task_id) {
            Ok(response) => {
                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&response).unwrap_or_else(|_| {
                            "{\"error\":\"failed to render JSON\"}".to_string()
                        })
                    );
                    return Ok(());
                }

                print!("{}", format_latest_approval(&response, task_id));
                return Ok(());
            }
            Err(error) if error.to_string().contains("status=404") => {
                if json_output {
                    return Err(error);
                }

                println!("No approval record found for task {}.", task_id);
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    if approve {
        let task_id =
            task_id.ok_or_else(|| anyhow!("--task-id is required when --approve is provided"))?;
        let response = submit_installation_task_approval(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            approved_by,
            reason,
            expires_at_utc,
        )?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_submitted_approval(&response));
        return Ok(());
    }

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

fn validate_command_mode(
    task_id: Option<&str>,
    readiness: bool,
    approvals: bool,
    approval: bool,
    approve: bool,
    status: Option<&str>,
    materialize: bool,
    approved_by: Option<&str>,
    expires_at_utc: Option<&str>,
) -> Result<()> {
    let selected_modes = [
        readiness && task_id.is_none(),
        readiness && task_id.is_some(),
        approvals,
        approval,
        approve,
        status.is_some(),
        materialize,
        task_id.is_some() && !approval && !approve && status.is_none(),
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();

    if selected_modes > 1 {
        return Err(anyhow!(
            "ambiguous installation-tasks mode; choose only one of default list, --materialize, --task-id, --status, --readiness, --approval, --approvals, or --approve"
        ));
    }

    if readiness && status.is_some() {
        return Err(anyhow!("--status cannot be combined with --readiness"));
    }

    if readiness && approve {
        return Err(anyhow!("--approve cannot be combined with --readiness"));
    }

    if readiness && approval {
        return Err(anyhow!("--approval cannot be combined with --readiness"));
    }

    if readiness && approvals {
        return Err(anyhow!("--approvals cannot be combined with --readiness"));
    }

    if approvals && task_id.is_some() {
        return Err(anyhow!("--task-id cannot be combined with --approvals"));
    }

    if approval && task_id.is_none() {
        return Err(anyhow!("--task-id is required when --approval is provided"));
    }

    if approve && task_id.is_none() {
        return Err(anyhow!("--task-id is required when --approve is provided"));
    }

    if approved_by.is_some() && !approve {
        return Err(anyhow!("--approved-by can only be used with --approve"));
    }

    if expires_at_utc.is_some() && !approve {
        return Err(anyhow!("--expires-at-utc can only be used with --approve"));
    }

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

    if response.tasks.iter().any(task_has_candidate_policy) {
        lines.push(String::new());
        lines.push(
            "Note: assisted execution is not implemented in this client command; this is informational only."
                .to_string(),
        );
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
    if task_has_candidate_policy(task) {
        lines.push(String::new());
        lines.push(
            "Note: assisted execution is not implemented in this client command; this is informational only."
                .to_string(),
        );
    }
    lines.join("\n")
}

fn format_readiness_list(
    response: &InstallationTaskReadinessList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.readiness.len() as u64);

    let mut lines = vec![
        format!(
            "Assisted installation readiness for node {}",
            effective_node_id
        ),
        format!("Project: {}", effective_project_id),
    ];

    if response.readiness.is_empty() {
        lines.push("No readiness decisions for this node.".to_string());
        return lines.join("\n");
    }

    lines.push(format!("Tasks: {}", count));
    lines.push(String::new());

    for (index, readiness) in response.readiness.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_readiness_block(readiness));
    }

    if response.readiness.iter().any(readiness_is_ready) {
        lines.push(String::new());
        lines.push(
            "Note: readiness is informational only. This client command did not execute installation, and safe_to_auto_execute remains false."
                .to_string(),
        );
    }

    lines.join("\n")
}

fn format_single_readiness(
    readiness: &InstallationTaskReadiness,
    requested_task_id: &str,
) -> String {
    let task_id = readiness.task_id.as_deref().unwrap_or(requested_task_id);
    let mut lines = vec![format!("Readiness for task {}", task_id)];
    lines.extend(format_readiness_detail_lines(readiness));

    if readiness_is_ready(readiness) {
        lines.push(String::new());
        lines.push(
            "Note: readiness is informational only. This client command did not execute installation, and safe_to_auto_execute remains false."
                .to_string(),
        );
    }

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

    match task.installation_policy.as_ref() {
        Some(policy) => {
            lines.push(format!(
                "  policy: {}",
                policy.decision.as_deref().unwrap_or("unknown")
            ));

            if let Some(policy_reason) = policy.reason.as_deref() {
                lines.push(format!("  policy reason: {}", policy_reason));
            }

            if let Some(allowed_future_modes) = policy.allowed_future_modes.as_ref() {
                if !allowed_future_modes.is_empty() {
                    lines.push(format!(
                        "  allowed future modes: {}",
                        allowed_future_modes.join(", ")
                    ));
                }
            }
        }
        None => lines.push("  policy: not provided by server".to_string()),
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

    if let Some(policy) = task.installation_policy.as_ref() {
        lines.push(format!(
            "  policy: {}",
            policy.decision.as_deref().unwrap_or("unknown")
        ));

        if let Some(policy_reason) = policy.reason.as_deref() {
            lines.push(format!("  policy reason: {}", policy_reason));
        }

        if let Some(allowed_future_modes) = policy.allowed_future_modes.as_ref() {
            if !allowed_future_modes.is_empty() {
                lines.push(format!(
                    "  allowed future modes: {}",
                    allowed_future_modes.join(", ")
                ));
            }
        }
    }

    lines.join("\n")
}

fn format_readiness_block(readiness: &InstallationTaskReadiness) -> Vec<String> {
    let label = readiness
        .readiness
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let mut lines = vec![format!("[{}]", label)];
    lines.extend(format_readiness_detail_lines(readiness));
    lines
}

fn format_readiness_detail_lines(readiness: &InstallationTaskReadiness) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(task_id) = readiness.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(readiness_value) = readiness.readiness.as_deref() {
        lines.push(format!("  readiness: {}", readiness_value));
    }

    if let Some(task_status) = readiness.task_status.as_deref() {
        lines.push(format!("  task status: {}", task_status));
    }

    if let Some(policy_decision) = readiness.policy_decision.as_deref() {
        lines.push(format!("  policy: {}", policy_decision));
    }

    if let Some(approval_status) = readiness.approval_status.as_deref() {
        lines.push(format!("  approval: {}", approval_status));
    }

    if let Some(approval_id) = readiness.approval_id.as_deref() {
        lines.push(format!("  approval_id: {}", approval_id));
    }

    if let Some(approved_at_utc) = readiness.approved_at_utc.as_deref() {
        lines.push(format!("  approved_at_utc: {}", approved_at_utc));
    }

    if let Some(expires_at_utc) = readiness.expires_at_utc.as_deref() {
        lines.push(format!("  expires_at_utc: {}", expires_at_utc));
    }

    if let Some(safe_to_auto_execute) = readiness.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = readiness.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines
}

fn format_approval_list(
    response: &InstallationTaskApprovalList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.approvals.len() as u64);

    let mut lines = vec![
        format!("Installation task approvals for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    if response.approvals.is_empty() {
        lines.push("No installation task approvals for this node.".to_string());
        return lines.join("\n");
    }

    lines.push(format!("Approvals: {}", count));
    lines.push(String::new());

    for (index, approval) in response.approvals.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_approval_block(approval));
    }

    lines.join("\n")
}

fn format_latest_approval(approval: &InstallationTaskApproval, task_id: &str) -> String {
    let mut lines = vec![format!("Latest approval for task {}", task_id)];

    if let Some(approved) = approval.approved {
        lines.push(format!("  approved: {}", approved));
    }

    if let Some(mode) = approval.approval_mode.as_deref() {
        lines.push(format!("  mode: {}", mode));
    }

    if let Some(approved_by) = approval.approved_by.as_deref() {
        lines.push(format!("  approved_by: {}", approved_by));
    }

    if let Some(approved_at_utc) = approval.approved_at_utc.as_deref() {
        lines.push(format!("  approved_at_utc: {}", approved_at_utc));
    }

    if let Some(reason) = approval.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines.join("\n")
}

fn format_submitted_approval(approval: &InstallationTaskApproval) -> String {
    let mut lines = vec!["Submitted installation task approval".to_string()];

    if let Some(task_id) = approval.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(approval_id) = approval.approval_id.as_deref() {
        lines.push(format!("  approval_id: {}", approval_id));
    }

    if let Some(approved) = approval.approved {
        lines.push(format!("  approved: {}", approved));
    }

    if let Some(mode) = approval.approval_mode.as_deref() {
        lines.push(format!("  mode: {}", mode));
    }

    if let Some(approved_by) = approval.approved_by.as_deref() {
        lines.push(format!("  approved_by: {}", approved_by));
    }

    lines.push(String::new());
    lines.push(
        "Note: approval metadata was recorded on the server. This client command did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn format_approval_block(approval: &InstallationTaskApproval) -> Vec<String> {
    let approved = if approval.approved == Some(true) {
        "APPROVED"
    } else {
        "UNKNOWN"
    };
    let mode = approval.approval_mode.as_deref().unwrap_or("unknown");
    let mut lines = vec![format!("[{}] {}", approved, mode)];

    if let Some(task_id) = approval.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(approval_id) = approval.approval_id.as_deref() {
        lines.push(format!("  approval_id: {}", approval_id));
    }

    if let Some(approved_by) = approval.approved_by.as_deref() {
        lines.push(format!("  approved_by: {}", approved_by));
    }

    if let Some(approved_at_utc) = approval.approved_at_utc.as_deref() {
        lines.push(format!("  approved_at_utc: {}", approved_at_utc));
    }

    if let Some(reason) = approval.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines
}

fn task_has_candidate_policy(task: &InstallationTask) -> bool {
    task.installation_policy
        .as_ref()
        .and_then(|policy| policy.decision.as_deref())
        == Some("candidate_for_assisted_execution")
}

fn readiness_is_ready(readiness: &InstallationTaskReadiness) -> bool {
    readiness.readiness.as_deref() == Some("ready_for_assisted_execution")
}

#[cfg(test)]
mod tests {
    use super::{
        format_approval_list, format_latest_approval, format_readiness_list,
        format_single_readiness, format_status_update, format_submitted_approval, format_task_list,
        validate_client_status, validate_command_mode,
    };
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, HttpConfig, OutputsConfig},
        installation_tasks::model::{
            InstallationPolicy, InstallationTask, InstallationTaskApproval,
            InstallationTaskApprovalList, InstallationTaskList, InstallationTaskReadiness,
            InstallationTaskReadinessList,
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
                    installation_policy: Some(InstallationPolicy {
                        decision: Some("manual_only".to_string()),
                        reason: Some(
                            "local trusted mapping requires operator confirmation".to_string(),
                        ),
                        ..InstallationPolicy::default()
                    }),
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
        assert!(rendered.contains("policy: manual_only"));
        assert!(rendered
            .contains("policy reason: local trusted mapping requires operator confirmation"));
        assert!(rendered.contains("reason: client lacks trusted local mapping for this action"));
    }

    #[test]
    fn formatting_includes_allowed_future_modes() {
        let rendered = format_task_list(
            &InstallationTaskList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                tasks: vec![InstallationTask {
                    task_id: Some("task-1".to_string()),
                    status: Some("planned".to_string()),
                    priority: Some("high".to_string()),
                    installation_action: Some(
                        "prepare_existing_artifact_installation".to_string(),
                    ),
                    installation_policy: Some(InstallationPolicy {
                        decision: Some("candidate_for_assisted_execution".to_string()),
                        reason: Some(
                            "artifact installation may be assisted in a future milestone, but is not executable yet".to_string(),
                        ),
                        allowed_future_modes: Some(vec!["operator_approved".to_string()]),
                        ..InstallationPolicy::default()
                    }),
                    ..InstallationTask::default()
                }],
                ..InstallationTaskList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("policy: candidate_for_assisted_execution"));
        assert!(rendered.contains(
            "policy reason: artifact installation may be assisted in a future milestone, but is not executable yet"
        ));
        assert!(rendered.contains("allowed future modes: operator_approved"));
    }

    #[test]
    fn candidate_policy_includes_informational_only_note() {
        let rendered = format_task_list(
            &InstallationTaskList {
                tasks: vec![InstallationTask {
                    installation_policy: Some(InstallationPolicy {
                        decision: Some("candidate_for_assisted_execution".to_string()),
                        ..InstallationPolicy::default()
                    }),
                    ..InstallationTask::default()
                }],
                ..InstallationTaskList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains(
            "Note: assisted execution is not implemented in this client command; this is informational only."
        ));
    }

    #[test]
    fn formatting_missing_policy_does_not_panic() {
        let rendered = format_task_list(
            &InstallationTaskList {
                tasks: vec![InstallationTask {
                    task_id: Some("task-1".to_string()),
                    ..InstallationTask::default()
                }],
                ..InstallationTaskList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("policy: not provided by server"));
    }

    #[test]
    fn status_patch_response_formatting_includes_policy_when_present() {
        let rendered = format_status_update(&InstallationTask {
            task_id: Some("task-1".to_string()),
            status: Some("acknowledged".to_string()),
            updated_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
            installation_policy: Some(InstallationPolicy {
                decision: Some("manual_only".to_string()),
                reason: Some("local trusted mapping requires operator confirmation".to_string()),
                ..InstallationPolicy::default()
            }),
            ..InstallationTask::default()
        });

        assert!(rendered.contains("policy: manual_only"));
        assert!(rendered
            .contains("policy reason: local trusted mapping requires operator confirmation"));
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

    #[test]
    fn approval_formatting_includes_key_fields() {
        let rendered = format_approval_list(
            &InstallationTaskApprovalList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                approvals: vec![InstallationTaskApproval {
                    task_id: Some("task-1".to_string()),
                    approval_id: Some("approval-1".to_string()),
                    approved: Some(true),
                    approved_by: Some("operator".to_string()),
                    approved_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                    approval_mode: Some("operator_approved".to_string()),
                    reason: Some("approved for future assisted execution".to_string()),
                    ..InstallationTaskApproval::default()
                }],
                ..InstallationTaskApprovalList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("approval_id: approval-1"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("approved_by: operator"));
        assert!(rendered.contains("approved_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("[APPROVED] operator_approved"));
        assert!(rendered.contains("reason: approved for future assisted execution"));
    }

    #[test]
    fn successful_submit_output_includes_metadata_only_note() {
        let rendered = format_submitted_approval(&InstallationTaskApproval {
            task_id: Some("task-1".to_string()),
            approval_id: Some("approval-1".to_string()),
            approved: Some(true),
            approval_mode: Some("operator_approved".to_string()),
            approved_by: Some("operator".to_string()),
            ..InstallationTaskApproval::default()
        });

        assert!(rendered.contains("Submitted installation task approval"));
        assert!(rendered.contains("approval_id: approval-1"));
        assert!(rendered.contains(
            "Note: approval metadata was recorded on the server. This client command did not execute installation."
        ));
    }

    #[test]
    fn latest_approval_formatting_includes_key_fields() {
        let rendered = format_latest_approval(
            &InstallationTaskApproval {
                approved: Some(true),
                approval_mode: Some("operator_approved".to_string()),
                approved_by: Some("operator".to_string()),
                approved_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                reason: Some("approved for future assisted execution".to_string()),
                ..InstallationTaskApproval::default()
            },
            "task-1",
        );

        assert!(rendered.contains("Latest approval for task task-1"));
        assert!(rendered.contains("approved: true"));
        assert!(rendered.contains("mode: operator_approved"));
        assert!(rendered.contains("approved_by: operator"));
        assert!(rendered.contains("approved_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("reason: approved for future assisted execution"));
    }

    #[test]
    fn readiness_formatting_includes_key_fields() {
        let rendered = format_readiness_list(
            &InstallationTaskReadinessList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                readiness: vec![InstallationTaskReadiness {
                    task_id: Some("task-1".to_string()),
                    readiness: Some("not_ready".to_string()),
                    task_status: Some("planned".to_string()),
                    policy_decision: Some("candidate_for_assisted_execution".to_string()),
                    approval_status: Some("missing".to_string()),
                    safe_to_auto_execute: Some(false),
                    reason: Some("operator approval is required".to_string()),
                    ..InstallationTaskReadiness::default()
                }],
                ..InstallationTaskReadinessList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("[NOT_READY]"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("task status: planned"));
        assert!(rendered.contains("policy: candidate_for_assisted_execution"));
        assert!(rendered.contains("approval: missing"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: operator approval is required"));
    }

    #[test]
    fn ready_readiness_includes_informational_only_note() {
        let rendered = format_single_readiness(
            &InstallationTaskReadiness {
                task_id: Some("task-1".to_string()),
                readiness: Some("ready_for_assisted_execution".to_string()),
                task_status: Some("acknowledged".to_string()),
                policy_decision: Some("candidate_for_assisted_execution".to_string()),
                approval_status: Some("approved".to_string()),
                safe_to_auto_execute: Some(false),
                reason: Some(
                    "task has candidate policy and operator approval metadata".to_string(),
                ),
                ..InstallationTaskReadiness::default()
            },
            "task-1",
        );

        assert!(rendered.contains("readiness: ready_for_assisted_execution"));
        assert!(rendered.contains(
            "Note: readiness is informational only. This client command did not execute installation, and safe_to_auto_execute remains false."
        ));
    }

    #[test]
    fn readiness_json_preserves_raw_response() {
        let response = InstallationTaskReadiness {
            task_id: Some("task-1".to_string()),
            readiness: Some("ready_for_assisted_execution".to_string()),
            safe_to_auto_execute: Some(false),
            ..InstallationTaskReadiness::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"task_id\": \"task-1\""));
        assert!(rendered.contains("\"readiness\": \"ready_for_assisted_execution\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        let result = validate_command_mode(
            Some("task-1"),
            false,
            false,
            true,
            true,
            None,
            false,
            Some("operator"),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_accepts_approval_mode() {
        let result = validate_command_mode(
            Some("task-1"),
            false,
            false,
            false,
            true,
            None,
            false,
            Some("operator"),
            Some("2026-06-01T00:00:00Z"),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn cli_mode_validation_rejects_readiness_with_approval_modes() {
        let result = validate_command_mode(
            Some("task-1"),
            true,
            false,
            true,
            false,
            None,
            false,
            None,
            None,
        );

        assert!(result.is_err());
    }
}
