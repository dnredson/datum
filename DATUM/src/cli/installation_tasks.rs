use anyhow::{anyhow, Result};

use crate::{
    assisted_execution::runner::{
        run_assisted_execution_preflight, AssistedExecutionPreflightReport,
    },
    config::Config,
    installation_tasks::{
        client::{
            fetch_assisted_execution_attempts_for_node, fetch_assisted_execution_claims_for_node,
            fetch_assisted_execution_requests_for_node, fetch_assisted_execution_state,
            fetch_assisted_execution_state_for_node, fetch_installation_task,
            fetch_installation_task_approvals_for_node, fetch_installation_task_readiness,
            fetch_installation_task_readiness_for_node, fetch_installation_tasks_for_node,
            fetch_latest_assisted_execution_attempt, fetch_latest_assisted_execution_claim,
            fetch_latest_assisted_execution_request, fetch_latest_installation_task_approval,
            materialize_installation_tasks_for_node, patch_assisted_execution_claim_status,
            patch_installation_task_status, submit_assisted_execution_attempt,
            submit_assisted_execution_claim, submit_assisted_execution_request,
            submit_installation_task_approval,
        },
        model::{
            AssistedExecutionAttempt, AssistedExecutionAttemptList, AssistedExecutionClaim,
            AssistedExecutionClaimList, AssistedExecutionRequest, AssistedExecutionRequestList,
            AssistedExecutionStateSummary, AssistedExecutionStateSummaryList, InstallationTask,
            InstallationTaskApproval, InstallationTaskApprovalList, InstallationTaskList,
            InstallationTaskReadiness, InstallationTaskReadinessList,
        },
    },
};

const ALLOWED_TASK_STATUSES: &[&str] = &["acknowledged", "blocked", "cancelled", "completed"];
const ALLOWED_CLAIM_STATUSES: &[&str] =
    &["claimed", "released", "expired", "completed", "cancelled"];
const ALLOWED_ATTEMPT_EXECUTION_MODES: &[&str] = &["no_op", "dry_run"];

pub fn run_installation_tasks_command(
    cfg: &Config,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    task_id: Option<&str>,
    request_id: Option<&str>,
    claim_id: Option<&str>,
    execution_state: bool,
    readiness: bool,
    execution_requests: bool,
    execution_request: bool,
    request_assisted_execution: bool,
    execution_claims: bool,
    execution_claim: bool,
    claim_assisted_execution: bool,
    claim_status: Option<&str>,
    execution_attempts: bool,
    execution_attempt: bool,
    attempt_assisted_execution: bool,
    execution_mode: Option<&str>,
    approvals: bool,
    approval: bool,
    approve: bool,
    approved_by: Option<&str>,
    requested_by: Option<&str>,
    claimed_by: Option<&str>,
    attempted_by: Option<&str>,
    message: Option<&str>,
    status: Option<&str>,
    reason: Option<&str>,
    expires_at_utc: Option<&str>,
    lease_expires_at_utc: Option<&str>,
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
        request_id,
        claim_id,
        execution_state,
        readiness,
        execution_requests,
        execution_request,
        request_assisted_execution,
        execution_claims,
        execution_claim,
        claim_assisted_execution,
        claim_status,
        execution_attempts,
        execution_attempt,
        attempt_assisted_execution,
        execution_mode,
        approvals,
        approval,
        approve,
        status,
        materialize,
        approved_by,
        requested_by,
        claimed_by,
        attempted_by,
        message,
        expires_at_utc,
        lease_expires_at_utc,
    )?;

    if execution_state {
        if let Some(task_id) = task_id {
            match fetch_assisted_execution_state(&base_url, &project_id, &node_id, task_id) {
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

                    print!(
                        "{}",
                        format_single_assisted_execution_state(&response, task_id)
                    );
                    return Ok(());
                }
                Err(error) if error.to_string().contains("status=404") => {
                    if json_output {
                        return Err(error);
                    }

                    println!("No assisted execution state found for task {}.", task_id);
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }

        let response = fetch_assisted_execution_state_for_node(&base_url, &project_id, &node_id)?;

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
            format_assisted_execution_state_list(&response, &project_id, &node_id)
        );
        return Ok(());
    }

    if execution_claims {
        let response = fetch_assisted_execution_claims_for_node(&base_url, &project_id, &node_id)?;

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
            format_assisted_execution_claim_list(&response, &project_id, &node_id)
        );
        return Ok(());
    }

    if execution_attempts {
        let response =
            fetch_assisted_execution_attempts_for_node(&base_url, &project_id, &node_id)?;

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
            format_assisted_execution_attempt_list(&response, &project_id, &node_id)
        );
        return Ok(());
    }

    if execution_claim {
        let task_id = task_id
            .ok_or_else(|| anyhow!("--task-id is required when --execution-claim is provided"))?;
        let request_id = request_id.ok_or_else(|| {
            anyhow!("--request-id is required when --execution-claim is provided")
        })?;
        match fetch_latest_assisted_execution_claim(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            request_id,
        ) {
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

                print!(
                    "{}",
                    format_latest_assisted_execution_claim(&response, task_id, request_id)
                );
                return Ok(());
            }
            Err(error) if error.to_string().contains("status=404") => {
                if json_output {
                    return Err(error);
                }

                println!(
                    "No assisted execution claim found for task {} request {}.",
                    task_id, request_id
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    if execution_attempt {
        let task_id = task_id
            .ok_or_else(|| anyhow!("--task-id is required when --execution-attempt is provided"))?;
        let request_id = request_id.ok_or_else(|| {
            anyhow!("--request-id is required when --execution-attempt is provided")
        })?;
        let claim_id = claim_id.ok_or_else(|| {
            anyhow!("--claim-id is required when --execution-attempt is provided")
        })?;
        match fetch_latest_assisted_execution_attempt(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            request_id,
            claim_id,
        ) {
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

                print!(
                    "{}",
                    format_latest_assisted_execution_attempt(
                        &response, task_id, request_id, claim_id
                    )
                );
                return Ok(());
            }
            Err(error) if error.to_string().contains("status=404") => {
                if json_output {
                    return Err(error);
                }

                println!(
                    "No assisted execution attempt found for task {} request {} claim {}.",
                    task_id, request_id, claim_id
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    if claim_assisted_execution {
        let task_id = task_id.ok_or_else(|| {
            anyhow!("--task-id is required when --claim-assisted-execution is provided")
        })?;
        let request_id = request_id.ok_or_else(|| {
            anyhow!("--request-id is required when --claim-assisted-execution is provided")
        })?;
        let response = submit_assisted_execution_claim(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            request_id,
            claimed_by,
            reason,
            lease_expires_at_utc,
        )?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_submitted_assisted_execution_claim(&response));
        return Ok(());
    }

    if attempt_assisted_execution {
        let task_id = task_id.ok_or_else(|| {
            anyhow!("--task-id is required when --attempt-assisted-execution is provided")
        })?;
        let request_id = request_id.ok_or_else(|| {
            anyhow!("--request-id is required when --attempt-assisted-execution is provided")
        })?;
        let claim_id = claim_id.ok_or_else(|| {
            anyhow!("--claim-id is required when --attempt-assisted-execution is provided")
        })?;
        let execution_mode = execution_mode.ok_or_else(|| {
            anyhow!("--execution-mode is required when --attempt-assisted-execution is provided")
        })?;
        let (preflight, attempt_message) = prepare_assisted_execution_attempt_submission(
            cfg,
            task_id,
            request_id,
            claim_id,
            execution_mode,
            message,
        )?;
        let response = submit_assisted_execution_attempt(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            request_id,
            claim_id,
            execution_mode,
            attempted_by,
            attempt_message.as_deref(),
        )?;

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
            format_submitted_assisted_execution_attempt_with_preflight(&preflight, &response)
        );
        return Ok(());
    }

    if let Some(claim_status) = claim_status {
        let task_id = task_id
            .ok_or_else(|| anyhow!("--task-id is required when --claim-status is provided"))?;
        let request_id = request_id
            .ok_or_else(|| anyhow!("--request-id is required when --claim-status is provided"))?;
        validate_client_claim_status(claim_status)?;
        let response = patch_assisted_execution_claim_status(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            request_id,
            claim_status,
            reason,
        )?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_updated_assisted_execution_claim(&response));
        return Ok(());
    }

    if execution_requests {
        let response =
            fetch_assisted_execution_requests_for_node(&base_url, &project_id, &node_id)?;

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
            format_assisted_execution_request_list(&response, &project_id, &node_id)
        );
        return Ok(());
    }

    if execution_request {
        let task_id = task_id
            .ok_or_else(|| anyhow!("--task-id is required when --execution-request is provided"))?;
        match fetch_latest_assisted_execution_request(&base_url, &project_id, &node_id, task_id) {
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

                print!(
                    "{}",
                    format_latest_assisted_execution_request(&response, task_id)
                );
                return Ok(());
            }
            Err(error) if error.to_string().contains("status=404") => {
                if json_output {
                    return Err(error);
                }

                println!("No assisted execution request found for task {}.", task_id);
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }

    if request_assisted_execution {
        let task_id = task_id.ok_or_else(|| {
            anyhow!("--task-id is required when --request-assisted-execution is provided")
        })?;
        let response = submit_assisted_execution_request(
            &base_url,
            &project_id,
            &node_id,
            task_id,
            requested_by,
            reason,
        )?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_submitted_assisted_execution_request(&response));
        return Ok(());
    }

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
    request_id: Option<&str>,
    claim_id: Option<&str>,
    execution_state: bool,
    readiness: bool,
    execution_requests: bool,
    execution_request: bool,
    request_assisted_execution: bool,
    execution_claims: bool,
    execution_claim: bool,
    claim_assisted_execution: bool,
    claim_status: Option<&str>,
    execution_attempts: bool,
    execution_attempt: bool,
    attempt_assisted_execution: bool,
    execution_mode: Option<&str>,
    approvals: bool,
    approval: bool,
    approve: bool,
    status: Option<&str>,
    materialize: bool,
    approved_by: Option<&str>,
    requested_by: Option<&str>,
    claimed_by: Option<&str>,
    attempted_by: Option<&str>,
    message: Option<&str>,
    expires_at_utc: Option<&str>,
    lease_expires_at_utc: Option<&str>,
) -> Result<()> {
    let selected_modes = [
        execution_state && task_id.is_none(),
        execution_state && task_id.is_some(),
        readiness && task_id.is_none(),
        readiness && task_id.is_some(),
        execution_requests,
        execution_request,
        request_assisted_execution,
        execution_claims,
        execution_claim,
        claim_assisted_execution,
        claim_status.is_some(),
        execution_attempts,
        execution_attempt,
        attempt_assisted_execution,
        approvals,
        approval,
        approve,
        status.is_some(),
        materialize,
        task_id.is_some()
            && !execution_state
            && !readiness
            && !execution_request
            && !request_assisted_execution
            && !execution_claim
            && !claim_assisted_execution
            && !execution_attempt
            && !attempt_assisted_execution
            && claim_status.is_none()
            && !approval
            && !approve
            && status.is_none(),
    ]
    .into_iter()
    .filter(|selected| *selected)
    .count();

    if selected_modes > 1 {
        return Err(anyhow!(
            "ambiguous installation-tasks mode; choose only one of default list, --materialize, --task-id, --status, --readiness, --execution-request, --execution-requests, --request-assisted-execution, --execution-claim, --execution-claims, --claim-assisted-execution, --claim-status, --execution-attempt, --execution-attempts, --attempt-assisted-execution, --approval, --approvals, or --approve"
        ));
    }

    if execution_state && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --execution-state"
        ));
    }

    if execution_state && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --execution-state"
        ));
    }

    if execution_requests && task_id.is_some() {
        return Err(anyhow!(
            "--task-id cannot be combined with --execution-requests"
        ));
    }

    if execution_request && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --execution-request is provided"
        ));
    }

    if execution_request && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --execution-request"
        ));
    }

    if request_assisted_execution && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --request-assisted-execution is provided"
        ));
    }

    if request_assisted_execution && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --request-assisted-execution"
        ));
    }

    if execution_claims && task_id.is_some() {
        return Err(anyhow!(
            "--task-id cannot be combined with --execution-claims"
        ));
    }

    if execution_claims && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --execution-claims"
        ));
    }

    if execution_claims && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --execution-claims"
        ));
    }

    if execution_claim && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --execution-claim is provided"
        ));
    }

    if execution_claim && request_id.is_none() {
        return Err(anyhow!(
            "--request-id is required when --execution-claim is provided"
        ));
    }

    if execution_claim && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --execution-claim"
        ));
    }

    if claim_assisted_execution && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --claim-assisted-execution is provided"
        ));
    }

    if claim_assisted_execution && request_id.is_none() {
        return Err(anyhow!(
            "--request-id is required when --claim-assisted-execution is provided"
        ));
    }

    if claim_assisted_execution && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --claim-assisted-execution"
        ));
    }

    if claim_status.is_some() && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --claim-status is provided"
        ));
    }

    if claim_status.is_some() && request_id.is_none() {
        return Err(anyhow!(
            "--request-id is required when --claim-status is provided"
        ));
    }

    if claim_status.is_some() && claim_id.is_some() {
        return Err(anyhow!("--claim-id cannot be combined with --claim-status"));
    }

    if execution_attempts && task_id.is_some() {
        return Err(anyhow!(
            "--task-id cannot be combined with --execution-attempts"
        ));
    }

    if execution_attempts && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --execution-attempts"
        ));
    }

    if execution_attempts && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --execution-attempts"
        ));
    }

    if execution_attempt && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --execution-attempt is provided"
        ));
    }

    if execution_attempt && request_id.is_none() {
        return Err(anyhow!(
            "--request-id is required when --execution-attempt is provided"
        ));
    }

    if execution_attempt && claim_id.is_none() {
        return Err(anyhow!(
            "--claim-id is required when --execution-attempt is provided"
        ));
    }

    if attempt_assisted_execution && task_id.is_none() {
        return Err(anyhow!(
            "--task-id is required when --attempt-assisted-execution is provided"
        ));
    }

    if attempt_assisted_execution && request_id.is_none() {
        return Err(anyhow!(
            "--request-id is required when --attempt-assisted-execution is provided"
        ));
    }

    if attempt_assisted_execution && claim_id.is_none() {
        return Err(anyhow!(
            "--claim-id is required when --attempt-assisted-execution is provided"
        ));
    }

    if attempt_assisted_execution && execution_mode.is_none() {
        return Err(anyhow!(
            "--execution-mode is required when --attempt-assisted-execution is provided"
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

    if requested_by.is_some() && !request_assisted_execution {
        return Err(anyhow!(
            "--requested-by can only be used with --request-assisted-execution"
        ));
    }

    if claimed_by.is_some() && !claim_assisted_execution {
        return Err(anyhow!(
            "--claimed-by can only be used with --claim-assisted-execution"
        ));
    }

    if claimed_by.is_some() && claim_status.is_some() {
        return Err(anyhow!(
            "--claimed-by cannot be combined with --claim-status"
        ));
    }

    if claimed_by.is_some() && attempt_assisted_execution {
        return Err(anyhow!(
            "--claimed-by cannot be combined with --attempt-assisted-execution"
        ));
    }

    if lease_expires_at_utc.is_some() && !claim_assisted_execution {
        return Err(anyhow!(
            "--lease-expires-at-utc can only be used with --claim-assisted-execution"
        ));
    }

    if lease_expires_at_utc.is_some() && claim_status.is_some() {
        return Err(anyhow!(
            "--lease-expires-at-utc cannot be combined with --claim-status"
        ));
    }

    if attempted_by.is_some() && !attempt_assisted_execution {
        return Err(anyhow!(
            "--attempted-by can only be used with --attempt-assisted-execution"
        ));
    }

    if message.is_some() && !attempt_assisted_execution {
        return Err(anyhow!(
            "--message can only be used with --attempt-assisted-execution"
        ));
    }

    if execution_mode.is_some() && !attempt_assisted_execution {
        return Err(anyhow!(
            "--execution-mode can only be used with --attempt-assisted-execution"
        ));
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
    if ALLOWED_TASK_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(anyhow!(
            "invalid installation task status '{}'; allowed values: {}",
            status,
            ALLOWED_TASK_STATUSES.join(", ")
        ))
    }
}

pub fn validate_client_claim_status(status: &str) -> Result<()> {
    if ALLOWED_CLAIM_STATUSES.contains(&status) {
        Ok(())
    } else {
        Err(anyhow!(
            "invalid assisted execution claim status '{}'; allowed values: {}",
            status,
            ALLOWED_CLAIM_STATUSES.join(", ")
        ))
    }
}

pub fn validate_client_attempt_execution_mode(execution_mode: &str) -> Result<()> {
    if ALLOWED_ATTEMPT_EXECUTION_MODES.contains(&execution_mode) {
        Ok(())
    } else {
        Err(anyhow!(
            "invalid assisted execution attempt mode '{}'; allowed values: {}",
            execution_mode,
            ALLOWED_ATTEMPT_EXECUTION_MODES.join(", ")
        ))
    }
}

fn prepare_assisted_execution_attempt_submission(
    cfg: &Config,
    task_id: &str,
    request_id: &str,
    claim_id: &str,
    execution_mode: &str,
    message: Option<&str>,
) -> Result<(AssistedExecutionPreflightReport, Option<String>)> {
    let preflight =
        run_assisted_execution_preflight(cfg, task_id, request_id, claim_id, execution_mode);

    if preflight.preflight_status == "failed" {
        return Err(anyhow!(
            "assisted execution preflight failed; attempt not submitted\n{}",
            format_assisted_execution_preflight_report(&preflight)
        ));
    }

    validate_client_attempt_execution_mode(&preflight.execution_mode)?;
    let attempt_message = build_attempt_message_with_preflight(message, &preflight);
    Ok((preflight, attempt_message))
}

fn build_attempt_message_with_preflight(
    message: Option<&str>,
    preflight: &AssistedExecutionPreflightReport,
) -> Option<String> {
    let trimmed = message.map(str::trim).filter(|value| !value.is_empty());
    let summary = format!("preflight: {}", preflight.message);

    match trimmed {
        Some(message) => Some(format!("{message}\n\n{summary}")),
        None => Some(summary),
    }
}

fn format_assisted_execution_preflight_report(report: &AssistedExecutionPreflightReport) -> String {
    let mut lines = vec!["Assisted execution preflight".to_string()];
    lines.push(format!("  mode: {}", report.execution_mode));
    lines.push(format!("  status: {}", report.preflight_status));
    lines.push(format!(
        "  safe to auto execute: {}",
        report.safe_to_auto_execute
    ));
    lines.push(format!(
        "  side effects performed: {}",
        report.side_effects_performed
    ));
    lines.push(String::new());
    lines.push("Checks:".to_string());

    for check in &report.checks {
        lines.push(format!(
            "  [{}] {}: {}",
            check.status, check.name, check.message
        ));
    }

    lines.join("\n")
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

fn format_assisted_execution_request_list(
    response: &AssistedExecutionRequestList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.requests.len() as u64);

    let mut lines = vec![
        format!("Assisted execution requests for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    if response.requests.is_empty() {
        lines.push("No assisted execution requests for this node.".to_string());
        return lines.join("\n");
    }

    lines.push(format!("Requests: {}", count));
    lines.push(String::new());

    for (index, request) in response.requests.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_assisted_execution_request_block(request));
    }

    lines.join("\n")
}

fn format_assisted_execution_claim_list(
    response: &AssistedExecutionClaimList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.claims.len() as u64);

    let mut lines = vec![
        format!("Assisted execution claims for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    if response.claims.is_empty() {
        lines.push("No assisted execution claims for this node.".to_string());
        return lines.join("\n");
    }

    lines.push(format!("Claims: {}", count));
    lines.push(String::new());

    for (index, claim) in response.claims.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_assisted_execution_claim_block(claim));
    }

    lines.join("\n")
}

fn format_assisted_execution_attempt_list(
    response: &AssistedExecutionAttemptList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.attempts.len() as u64);

    let mut lines = vec![
        format!("Assisted execution attempts for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    if response.attempts.is_empty() {
        lines.push("No assisted execution attempts for this node.".to_string());
        return lines.join("\n");
    }

    lines.push(format!("Attempts: {}", count));
    lines.push(String::new());

    for (index, attempt) in response.attempts.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_assisted_execution_attempt_block(attempt));
    }

    lines.join("\n")
}

fn format_assisted_execution_state_list(
    response: &AssistedExecutionStateSummaryList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.summaries.len() as u64);

    let mut lines = vec![
        format!("Assisted execution state for node {}", effective_node_id),
        format!("Project: {}", effective_project_id),
    ];

    if response.summaries.is_empty() {
        lines.push("No assisted execution state summaries for this node.".to_string());
        return lines.join("\n");
    }

    lines.push(format!("Summaries: {}", count));
    lines.push(String::new());

    for (index, summary) in response.summaries.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_assisted_execution_state_block(summary));
    }

    lines.join("\n")
}

fn format_latest_assisted_execution_request(
    request: &AssistedExecutionRequest,
    requested_task_id: &str,
) -> String {
    let task_id = request.task_id.as_deref().unwrap_or(requested_task_id);
    let mut lines = vec![format!(
        "Latest assisted execution request for task {}",
        task_id
    )];

    if let Some(status) = request.request_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(mode) = request.execution_mode.as_deref() {
        lines.push(format!("  mode: {}", mode));
    }

    if let Some(requested_by) = request.requested_by.as_deref() {
        lines.push(format!("  requested_by: {}", requested_by));
    }

    if let Some(requested_at_utc) = request.requested_at_utc.as_deref() {
        lines.push(format!("  requested_at_utc: {}", requested_at_utc));
    }

    if let Some(safe_to_auto_execute) = request.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = request.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines.join("\n")
}

fn format_single_assisted_execution_state(
    summary: &AssistedExecutionStateSummary,
    requested_task_id: &str,
) -> String {
    let task_id = summary.task_id.as_deref().unwrap_or(requested_task_id);
    let mut lines = vec![format!("Assisted execution state for task {}", task_id)];
    lines.extend(format_assisted_execution_state_detail_lines(summary));

    if assisted_execution_state_has_attempt(summary) {
        lines.push(String::new());
        lines.push(
            "Note: this state is informational. No installation execution is performed by this command."
                .to_string(),
        );
    }

    lines.join("\n")
}

fn format_latest_assisted_execution_claim(
    claim: &AssistedExecutionClaim,
    requested_task_id: &str,
    requested_request_id: &str,
) -> String {
    let mut lines = vec!["Latest assisted execution claim".to_string()];

    lines.push(format!(
        "  task_id: {}",
        claim.task_id.as_deref().unwrap_or(requested_task_id)
    ));
    lines.push(format!(
        "  request_id: {}",
        claim.request_id.as_deref().unwrap_or(requested_request_id)
    ));

    if let Some(claim_id) = claim.claim_id.as_deref() {
        lines.push(format!("  claim_id: {}", claim_id));
    }

    if let Some(status) = claim.claim_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(claimed_by) = claim.claimed_by.as_deref() {
        lines.push(format!("  claimed_by: {}", claimed_by));
    }

    if let Some(claimed_at_utc) = claim.claimed_at_utc.as_deref() {
        lines.push(format!("  claimed_at_utc: {}", claimed_at_utc));
    }

    if let Some(lease_expires_at_utc) = claim.lease_expires_at_utc.as_deref() {
        lines.push(format!("  lease_expires_at_utc: {}", lease_expires_at_utc));
    }

    if let Some(safe_to_auto_execute) = claim.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = claim.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines.join("\n")
}

fn format_assisted_execution_state_block(summary: &AssistedExecutionStateSummary) -> Vec<String> {
    let label = summary
        .state
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let mut lines = vec![format!("[{}]", label)];
    lines.extend(format_assisted_execution_state_detail_lines(summary));

    if assisted_execution_state_has_attempt(summary) {
        lines.push(
            "  Note: this state is informational. No installation execution is performed by this command."
                .to_string(),
        );
    }

    lines
}

fn format_assisted_execution_state_detail_lines(
    summary: &AssistedExecutionStateSummary,
) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(task_id) = summary.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(state) = summary.state.as_deref() {
        lines.push(format!("  state: {}", state));
    }

    if let Some(task_status) = summary.task_status.as_deref() {
        lines.push(format!("  task status: {}", task_status));
    }

    if let Some(policy_decision) = summary.policy_decision.as_deref() {
        lines.push(format!("  policy: {}", policy_decision));
    }

    if let Some(approval_status) = summary.approval_status.as_deref() {
        lines.push(format!("  approval: {}", approval_status));
    }

    if let Some(readiness) = summary.readiness.as_deref() {
        lines.push(format!("  readiness: {}", readiness));
    }

    if let Some(request_status) = summary.request_status.as_deref() {
        lines.push(format!("  request: {}", request_status));
    }

    if let Some(claim_status) = summary.claim_status.as_deref() {
        lines.push(format!("  claim: {}", claim_status));
    }

    if let Some(latest_attempt_status) = summary.latest_attempt_status.as_deref() {
        lines.push(format!("  latest attempt: {}", latest_attempt_status));
    }

    if let Some(latest_execution_mode) = summary.latest_execution_mode.as_deref() {
        lines.push(format!("  latest mode: {}", latest_execution_mode));
    }

    if let Some(latest_execution_result) = summary.latest_execution_result.as_deref() {
        lines.push(format!("  latest result: {}", latest_execution_result));
    }

    lines.push(format!(
        "  safe to auto execute: {}",
        summary.safe_to_auto_execute.unwrap_or(false)
    ));
    lines.push(format!(
        "  side effects performed: {}",
        summary.side_effects_performed.unwrap_or(false)
    ));

    if let Some(reason) = summary.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines
}

fn assisted_execution_state_has_attempt(summary: &AssistedExecutionStateSummary) -> bool {
    matches!(
        summary.state.as_deref(),
        Some("attempt_recorded_no_op") | Some("attempt_recorded_dry_run")
    ) || summary.latest_attempt_status.is_some()
}

fn format_latest_assisted_execution_attempt(
    attempt: &AssistedExecutionAttempt,
    requested_task_id: &str,
    requested_request_id: &str,
    requested_claim_id: &str,
) -> String {
    let mut lines = vec!["Latest assisted execution attempt".to_string()];

    lines.push(format!(
        "  task_id: {}",
        attempt.task_id.as_deref().unwrap_or(requested_task_id)
    ));
    lines.push(format!(
        "  request_id: {}",
        attempt
            .request_id
            .as_deref()
            .unwrap_or(requested_request_id)
    ));
    lines.push(format!(
        "  claim_id: {}",
        attempt.claim_id.as_deref().unwrap_or(requested_claim_id)
    ));

    if let Some(attempt_id) = attempt.attempt_id.as_deref() {
        lines.push(format!("  attempt_id: {}", attempt_id));
    }

    if let Some(status) = attempt.attempt_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(mode) = attempt.execution_mode.as_deref() {
        lines.push(format!("  mode: {}", mode));
    }

    if let Some(result) = attempt.execution_result.as_deref() {
        lines.push(format!("  result: {}", result));
    }

    if let Some(attempted_by) = attempt.attempted_by.as_deref() {
        lines.push(format!("  attempted_by: {}", attempted_by));
    }

    if let Some(attempted_at_utc) = attempt.attempted_at_utc.as_deref() {
        lines.push(format!("  attempted_at_utc: {}", attempted_at_utc));
    }

    if let Some(safe_to_auto_execute) = attempt.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(side_effects_performed) = attempt.side_effects_performed {
        lines.push(format!(
            "  side effects performed: {}",
            side_effects_performed
        ));
    }

    if let Some(message) = attempt.message.as_deref() {
        lines.push(format!("  message: {}", message));
    }

    lines.join("\n")
}

fn format_submitted_assisted_execution_request(request: &AssistedExecutionRequest) -> String {
    let mut lines = vec!["Submitted assisted execution request".to_string()];

    if let Some(task_id) = request.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = request.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(status) = request.request_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(mode) = request.execution_mode.as_deref() {
        lines.push(format!("  mode: {}", mode));
    }

    if let Some(requested_by) = request.requested_by.as_deref() {
        lines.push(format!("  requested_by: {}", requested_by));
    }

    if let Some(safe_to_auto_execute) = request.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    lines.push(String::new());
    lines.push(
        "Note: request metadata was recorded on the server. This client command did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn format_submitted_assisted_execution_claim(claim: &AssistedExecutionClaim) -> String {
    let mut lines = vec!["Submitted assisted execution claim".to_string()];

    if let Some(task_id) = claim.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = claim.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(claim_id) = claim.claim_id.as_deref() {
        lines.push(format!("  claim_id: {}", claim_id));
    }

    if let Some(status) = claim.claim_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(claimed_by) = claim.claimed_by.as_deref() {
        lines.push(format!("  claimed_by: {}", claimed_by));
    }

    if let Some(safe_to_auto_execute) = claim.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    lines.push(String::new());
    lines.push(
        "Note: claim metadata was recorded on the server. This client command did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn format_submitted_assisted_execution_attempt(attempt: &AssistedExecutionAttempt) -> String {
    let mut lines = vec!["Submitted assisted execution attempt".to_string()];

    if let Some(task_id) = attempt.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = attempt.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(claim_id) = attempt.claim_id.as_deref() {
        lines.push(format!("  claim_id: {}", claim_id));
    }

    if let Some(attempt_id) = attempt.attempt_id.as_deref() {
        lines.push(format!("  attempt_id: {}", attempt_id));
    }

    if let Some(status) = attempt.attempt_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(mode) = attempt.execution_mode.as_deref() {
        lines.push(format!("  mode: {}", mode));
    }

    if let Some(result) = attempt.execution_result.as_deref() {
        lines.push(format!("  result: {}", result));
    }

    if let Some(safe_to_auto_execute) = attempt.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(side_effects_performed) = attempt.side_effects_performed {
        lines.push(format!(
            "  side effects performed: {}",
            side_effects_performed
        ));
    }

    lines.push(String::new());
    lines.push(
        "Note: this client command performed preflight/no-op only, recorded attempt metadata on the server, and did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn format_submitted_assisted_execution_attempt_with_preflight(
    preflight: &AssistedExecutionPreflightReport,
    attempt: &AssistedExecutionAttempt,
) -> String {
    [
        format_assisted_execution_preflight_report(preflight),
        String::new(),
        format_submitted_assisted_execution_attempt(attempt),
    ]
    .join("\n")
}

fn format_updated_assisted_execution_claim(claim: &AssistedExecutionClaim) -> String {
    let mut lines = vec!["Updated assisted execution claim".to_string()];

    if let Some(task_id) = claim.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = claim.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(claim_id) = claim.claim_id.as_deref() {
        lines.push(format!("  claim_id: {}", claim_id));
    }

    if let Some(status) = claim.claim_status.as_deref() {
        lines.push(format!("  status: {}", status));
    }

    if let Some(claimed_by) = claim.claimed_by.as_deref() {
        lines.push(format!("  claimed_by: {}", claimed_by));
    }

    if let Some(safe_to_auto_execute) = claim.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = claim.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines.push(String::new());
    lines.push(
        "Note: claim metadata was updated on the server. This client command did not execute installation."
            .to_string(),
    );

    lines.join("\n")
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

fn format_assisted_execution_request_block(request: &AssistedExecutionRequest) -> Vec<String> {
    let status = request
        .request_status
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let mode = request.execution_mode.as_deref().unwrap_or("unknown");
    let mut lines = vec![format!("[{}] {}", status, mode)];

    if let Some(task_id) = request.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = request.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(requested_by) = request.requested_by.as_deref() {
        lines.push(format!("  requested_by: {}", requested_by));
    }

    if let Some(requested_at_utc) = request.requested_at_utc.as_deref() {
        lines.push(format!("  requested_at_utc: {}", requested_at_utc));
    }

    if let Some(safe_to_auto_execute) = request.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = request.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines
}

fn format_assisted_execution_claim_block(claim: &AssistedExecutionClaim) -> Vec<String> {
    let status = claim
        .claim_status
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let mut lines = vec![format!("[{}]", status)];

    if let Some(task_id) = claim.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = claim.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(claim_id) = claim.claim_id.as_deref() {
        lines.push(format!("  claim_id: {}", claim_id));
    }

    if let Some(claimed_by) = claim.claimed_by.as_deref() {
        lines.push(format!("  claimed_by: {}", claimed_by));
    }

    if let Some(claimed_at_utc) = claim.claimed_at_utc.as_deref() {
        lines.push(format!("  claimed_at_utc: {}", claimed_at_utc));
    }

    if let Some(lease_expires_at_utc) = claim.lease_expires_at_utc.as_deref() {
        lines.push(format!("  lease_expires_at_utc: {}", lease_expires_at_utc));
    }

    if let Some(safe_to_auto_execute) = claim.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(reason) = claim.reason.as_deref() {
        lines.push(format!("  reason: {}", reason));
    }

    lines
}

fn format_assisted_execution_attempt_block(attempt: &AssistedExecutionAttempt) -> Vec<String> {
    let status = attempt
        .attempt_status
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let mode = attempt.execution_mode.as_deref().unwrap_or("unknown");
    let mut lines = vec![format!("[{}] {}", status, mode)];

    if let Some(task_id) = attempt.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }

    if let Some(request_id) = attempt.request_id.as_deref() {
        lines.push(format!("  request_id: {}", request_id));
    }

    if let Some(claim_id) = attempt.claim_id.as_deref() {
        lines.push(format!("  claim_id: {}", claim_id));
    }

    if let Some(attempt_id) = attempt.attempt_id.as_deref() {
        lines.push(format!("  attempt_id: {}", attempt_id));
    }

    if let Some(attempted_by) = attempt.attempted_by.as_deref() {
        lines.push(format!("  attempted_by: {}", attempted_by));
    }

    if let Some(attempted_at_utc) = attempt.attempted_at_utc.as_deref() {
        lines.push(format!("  attempted_at_utc: {}", attempted_at_utc));
    }

    if let Some(execution_result) = attempt.execution_result.as_deref() {
        lines.push(format!("  execution_result: {}", execution_result));
    }

    if let Some(safe_to_auto_execute) = attempt.safe_to_auto_execute {
        lines.push(format!("  safe to auto execute: {}", safe_to_auto_execute));
    }

    if let Some(side_effects_performed) = attempt.side_effects_performed {
        lines.push(format!(
            "  side effects performed: {}",
            side_effects_performed
        ));
    }

    if let Some(message) = attempt.message.as_deref() {
        lines.push(format!("  message: {}", message));
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
        assisted_execution_state_has_attempt, build_attempt_message_with_preflight,
        format_approval_list, format_assisted_execution_attempt_list,
        format_assisted_execution_claim_list, format_assisted_execution_preflight_report,
        format_assisted_execution_request_list, format_assisted_execution_state_list,
        format_latest_approval, format_latest_assisted_execution_attempt,
        format_latest_assisted_execution_claim, format_latest_assisted_execution_request,
        format_readiness_list, format_single_assisted_execution_state, format_single_readiness,
        format_status_update, format_submitted_approval,
        format_submitted_assisted_execution_attempt,
        format_submitted_assisted_execution_attempt_with_preflight,
        format_submitted_assisted_execution_claim, format_submitted_assisted_execution_request,
        format_task_list, format_updated_assisted_execution_claim,
        prepare_assisted_execution_attempt_submission, validate_client_attempt_execution_mode,
        validate_client_claim_status, validate_client_status, validate_command_mode,
    };
    use crate::{
        assisted_execution::runner::{
            AssistedExecutionPreflightCheck, AssistedExecutionPreflightReport,
        },
        config::{
            AgentConfig, AssistedExecutionConfig, Config, DispatchConfig, HttpConfig, OutputsConfig,
        },
        installation_tasks::model::{
            AssistedExecutionAttempt, AssistedExecutionAttemptList, AssistedExecutionClaim,
            AssistedExecutionClaimList, AssistedExecutionRequest, AssistedExecutionRequestList,
            AssistedExecutionStateSummary, AssistedExecutionStateSummaryList, InstallationPolicy,
            InstallationTask, InstallationTaskApproval, InstallationTaskApprovalList,
            InstallationTaskList, InstallationTaskReadiness, InstallationTaskReadinessList,
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
    fn client_claim_status_validation_accepts_allowed_values() {
        assert!(validate_client_claim_status("claimed").is_ok());
        assert!(validate_client_claim_status("released").is_ok());
        assert!(validate_client_claim_status("expired").is_ok());
        assert!(validate_client_claim_status("completed").is_ok());
        assert!(validate_client_claim_status("cancelled").is_ok());
    }

    #[test]
    fn client_claim_status_validation_rejects_arbitrary_values() {
        assert!(validate_client_claim_status("requested").is_err());
        assert!(validate_client_claim_status("explode").is_err());
    }

    #[test]
    fn client_attempt_execution_mode_validation_accepts_allowed_values() {
        assert!(validate_client_attempt_execution_mode("no_op").is_ok());
        assert!(validate_client_attempt_execution_mode("dry_run").is_ok());
    }

    #[test]
    fn client_attempt_execution_mode_validation_rejects_arbitrary_values() {
        assert!(validate_client_attempt_execution_mode("execute").is_err());
        assert!(validate_client_attempt_execution_mode("explode").is_err());
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
    fn assisted_execution_request_formatting_includes_key_fields() {
        let rendered = format_assisted_execution_request_list(
            &AssistedExecutionRequestList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                requests: vec![AssistedExecutionRequest {
                    task_id: Some("task-1".to_string()),
                    request_id: Some("request-1".to_string()),
                    request_status: Some("requested".to_string()),
                    requested_by: Some("operator".to_string()),
                    requested_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                    execution_mode: Some("assisted_operator_approved".to_string()),
                    safe_to_auto_execute: Some(false),
                    reason: Some("request future assisted execution".to_string()),
                    ..AssistedExecutionRequest::default()
                }],
                ..AssistedExecutionRequestList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("[REQUESTED] assisted_operator_approved"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains("requested_by: operator"));
        assert!(rendered.contains("requested_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: request future assisted execution"));
    }

    #[test]
    fn successful_submit_request_output_includes_metadata_only_note() {
        let rendered = format_submitted_assisted_execution_request(&AssistedExecutionRequest {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            request_status: Some("requested".to_string()),
            execution_mode: Some("assisted_operator_approved".to_string()),
            requested_by: Some("operator".to_string()),
            safe_to_auto_execute: Some(false),
            ..AssistedExecutionRequest::default()
        });

        assert!(rendered.contains("Submitted assisted execution request"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains(
            "Note: request metadata was recorded on the server. This client command did not execute installation."
        ));
    }

    #[test]
    fn latest_assisted_execution_request_formatting_includes_key_fields() {
        let rendered = format_latest_assisted_execution_request(
            &AssistedExecutionRequest {
                request_status: Some("requested".to_string()),
                execution_mode: Some("assisted_operator_approved".to_string()),
                requested_by: Some("operator".to_string()),
                requested_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                safe_to_auto_execute: Some(false),
                reason: Some("request future assisted execution".to_string()),
                ..AssistedExecutionRequest::default()
            },
            "task-1",
        );

        assert!(rendered.contains("Latest assisted execution request for task task-1"));
        assert!(rendered.contains("status: requested"));
        assert!(rendered.contains("mode: assisted_operator_approved"));
        assert!(rendered.contains("requested_by: operator"));
        assert!(rendered.contains("requested_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: request future assisted execution"));
    }

    #[test]
    fn assisted_execution_request_json_preserves_raw_response() {
        let response = AssistedExecutionRequest {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            request_status: Some("requested".to_string()),
            safe_to_auto_execute: Some(false),
            ..AssistedExecutionRequest::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"task_id\": \"task-1\""));
        assert!(rendered.contains("\"request_id\": \"request-1\""));
        assert!(rendered.contains("\"request_status\": \"requested\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
    }

    #[test]
    fn claim_formatting_includes_key_fields() {
        let rendered = format_assisted_execution_claim_list(
            &AssistedExecutionClaimList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                claims: vec![AssistedExecutionClaim {
                    task_id: Some("task-1".to_string()),
                    request_id: Some("request-1".to_string()),
                    claim_id: Some("claim-1".to_string()),
                    claim_status: Some("claimed".to_string()),
                    claimed_by: Some("operator".to_string()),
                    claimed_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                    safe_to_auto_execute: Some(false),
                    reason: Some("claim metadata for future assisted execution".to_string()),
                    ..AssistedExecutionClaim::default()
                }],
                ..AssistedExecutionClaimList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("[CLAIMED]"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains("claimed_by: operator"));
        assert!(rendered.contains("claimed_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: claim metadata for future assisted execution"));
    }

    #[test]
    fn successful_submit_claim_output_includes_metadata_only_note() {
        let rendered = format_submitted_assisted_execution_claim(&AssistedExecutionClaim {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            claim_id: Some("claim-1".to_string()),
            claim_status: Some("claimed".to_string()),
            claimed_by: Some("operator".to_string()),
            safe_to_auto_execute: Some(false),
            ..AssistedExecutionClaim::default()
        });

        assert!(rendered.contains("Submitted assisted execution claim"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains(
            "Note: claim metadata was recorded on the server. This client command did not execute installation."
        ));
    }

    #[test]
    fn latest_claim_formatting_includes_key_fields() {
        let rendered = format_latest_assisted_execution_claim(
            &AssistedExecutionClaim {
                task_id: Some("task-1".to_string()),
                request_id: Some("request-1".to_string()),
                claim_id: Some("claim-1".to_string()),
                claim_status: Some("claimed".to_string()),
                claimed_by: Some("operator".to_string()),
                claimed_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                safe_to_auto_execute: Some(false),
                reason: Some("claim metadata for future assisted execution".to_string()),
                ..AssistedExecutionClaim::default()
            },
            "task-1",
            "request-1",
        );

        assert!(rendered.contains("Latest assisted execution claim"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains("status: claimed"));
        assert!(rendered.contains("claimed_by: operator"));
        assert!(rendered.contains("claimed_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: claim metadata for future assisted execution"));
    }

    #[test]
    fn claim_json_preserves_raw_response() {
        let response = AssistedExecutionClaim {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            claim_id: Some("claim-1".to_string()),
            claim_status: Some("claimed".to_string()),
            safe_to_auto_execute: Some(false),
            ..AssistedExecutionClaim::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"task_id\": \"task-1\""));
        assert!(rendered.contains("\"request_id\": \"request-1\""));
        assert!(rendered.contains("\"claim_id\": \"claim-1\""));
        assert!(rendered.contains("\"claim_status\": \"claimed\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
    }

    #[test]
    fn updated_claim_formatting_includes_key_fields() {
        let rendered = format_updated_assisted_execution_claim(&AssistedExecutionClaim {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            claim_id: Some("claim-1".to_string()),
            claim_status: Some("released".to_string()),
            claimed_by: Some("operator".to_string()),
            safe_to_auto_execute: Some(false),
            reason: Some("operator released claim".to_string()),
            ..AssistedExecutionClaim::default()
        });

        assert!(rendered.contains("Updated assisted execution claim"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains("status: released"));
        assert!(rendered.contains("claimed_by: operator"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("reason: operator released claim"));
    }

    #[test]
    fn successful_patch_output_includes_metadata_only_note() {
        let rendered = format_updated_assisted_execution_claim(&AssistedExecutionClaim {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            claim_id: Some("claim-1".to_string()),
            claim_status: Some("released".to_string()),
            claimed_by: Some("operator".to_string()),
            safe_to_auto_execute: Some(false),
            ..AssistedExecutionClaim::default()
        });

        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains(
            "Note: claim metadata was updated on the server. This client command did not execute installation."
        ));
    }

    #[test]
    fn attempt_formatting_includes_key_fields() {
        let rendered = format_assisted_execution_attempt_list(
            &AssistedExecutionAttemptList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                attempts: vec![AssistedExecutionAttempt {
                    task_id: Some("task-1".to_string()),
                    request_id: Some("request-1".to_string()),
                    claim_id: Some("claim-1".to_string()),
                    attempt_id: Some("attempt-1".to_string()),
                    attempt_status: Some("recorded".to_string()),
                    attempted_by: Some("operator".to_string()),
                    attempted_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                    execution_mode: Some("no_op".to_string()),
                    execution_result: Some("no_op_recorded".to_string()),
                    safe_to_auto_execute: Some(false),
                    side_effects_performed: Some(false),
                    message: Some("recording no-op attempt".to_string()),
                    ..AssistedExecutionAttempt::default()
                }],
                ..AssistedExecutionAttemptList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("[RECORDED] no_op"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains("attempt_id: attempt-1"));
        assert!(rendered.contains("attempted_by: operator"));
        assert!(rendered.contains("attempted_at_utc: 2026-05-19T13:00:00Z"));
        assert!(rendered.contains("execution_result: no_op_recorded"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("message: recording no-op attempt"));
    }

    #[test]
    fn latest_attempt_formatting_includes_key_fields() {
        let rendered = format_latest_assisted_execution_attempt(
            &AssistedExecutionAttempt {
                task_id: Some("task-1".to_string()),
                request_id: Some("request-1".to_string()),
                claim_id: Some("claim-1".to_string()),
                attempt_id: Some("attempt-1".to_string()),
                attempt_status: Some("recorded".to_string()),
                attempted_by: Some("operator".to_string()),
                attempted_at_utc: Some("2026-05-19T13:00:00Z".to_string()),
                execution_mode: Some("no_op".to_string()),
                execution_result: Some("no_op_recorded".to_string()),
                safe_to_auto_execute: Some(false),
                side_effects_performed: Some(false),
                message: Some("recording no-op attempt".to_string()),
                ..AssistedExecutionAttempt::default()
            },
            "task-1",
            "request-1",
            "claim-1",
        );

        assert!(rendered.contains("Latest assisted execution attempt"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("request_id: request-1"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains("attempt_id: attempt-1"));
        assert!(rendered.contains("status: recorded"));
        assert!(rendered.contains("mode: no_op"));
        assert!(rendered.contains("result: no_op_recorded"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("message: recording no-op attempt"));
    }

    #[test]
    fn successful_submit_attempt_output_includes_metadata_only_note() {
        let rendered = format_submitted_assisted_execution_attempt(&AssistedExecutionAttempt {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            claim_id: Some("claim-1".to_string()),
            attempt_id: Some("attempt-1".to_string()),
            attempt_status: Some("recorded".to_string()),
            execution_mode: Some("no_op".to_string()),
            execution_result: Some("no_op_recorded".to_string()),
            safe_to_auto_execute: Some(false),
            side_effects_performed: Some(false),
            ..AssistedExecutionAttempt::default()
        });

        assert!(rendered.contains("Submitted assisted execution attempt"));
        assert!(rendered.contains("attempt_id: attempt-1"));
        assert!(rendered.contains("claim_id: claim-1"));
        assert!(rendered.contains("mode: no_op"));
        assert!(rendered.contains("result: no_op_recorded"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains(
            "Note: this client command performed preflight/no-op only, recorded attempt metadata on the server, and did not execute installation."
        ));
    }

    #[test]
    fn attempt_preflight_formatting_includes_expected_fields() {
        let rendered = format_assisted_execution_preflight_report(&AssistedExecutionPreflightReport {
            task_id: "task-1".to_string(),
            request_id: "request-1".to_string(),
            claim_id: "claim-1".to_string(),
            execution_mode: "no_op".to_string(),
            local_node_id: "fog-01".to_string(),
            preflight_status: "warning".to_string(),
            checks: vec![
                AssistedExecutionPreflightCheck {
                    name: "execution_mode_allowed".to_string(),
                    status: "passed".to_string(),
                    message: "no_op is allowed".to_string(),
                },
                AssistedExecutionPreflightCheck {
                    name: "assisted_execution_enabled_config_check".to_string(),
                    status: "warning".to_string(),
                    message: "assisted execution is not enabled; metadata-only preflight/no-op attempt only".to_string(),
                },
            ],
            message: "preflight_status=warning checks=2 warnings=1 failures=0 local_node_id=fog-01 execution_mode=no_op safe_to_auto_execute=false side_effects_performed=false".to_string(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        });

        assert!(rendered.contains("Assisted execution preflight"));
        assert!(rendered.contains("mode: no_op"));
        assert!(rendered.contains("status: warning"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("[passed] execution_mode_allowed: no_op is allowed"));
        assert!(rendered.contains("[warning] assisted_execution_enabled_config_check"));
    }

    #[test]
    fn attempt_message_appends_preflight_summary() {
        let preflight = AssistedExecutionPreflightReport {
            task_id: "task-1".to_string(),
            request_id: "request-1".to_string(),
            claim_id: "claim-1".to_string(),
            execution_mode: "no_op".to_string(),
            local_node_id: "fog-01".to_string(),
            preflight_status: "passed".to_string(),
            checks: vec![],
            message: "preflight_status=passed checks=4 warnings=0 failures=0 local_node_id=fog-01 execution_mode=no_op safe_to_auto_execute=false side_effects_performed=false".to_string(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        };

        let combined = build_attempt_message_with_preflight(
            Some("manual preflight no-op attempt"),
            &preflight,
        )
        .expect("message should exist");

        assert!(combined.contains("manual preflight no-op attempt"));
        assert!(combined.contains("preflight: preflight_status=passed"));
    }

    #[test]
    fn failed_preflight_prevents_attempt_submission() {
        let mut cfg = base_config();
        cfg.assisted_execution = Some(AssistedExecutionConfig {
            enabled: true,
            allow_no_op_attempts: false,
            allow_dry_run_attempts: true,
            allow_side_effects: false,
        });

        let err = prepare_assisted_execution_attempt_submission(
            &cfg,
            "task-1",
            "request-1",
            "claim-1",
            "no_op",
            Some("manual preflight no-op attempt"),
        )
        .expect_err("preflight should fail");

        let rendered = err.to_string();
        assert!(rendered.contains("assisted execution preflight failed; attempt not submitted"));
        assert!(rendered.contains("execution_mode_allowed"));
    }

    #[test]
    fn successful_attempt_output_includes_preflight_and_no_execution_note() {
        let preflight = AssistedExecutionPreflightReport {
            task_id: "task-1".to_string(),
            request_id: "request-1".to_string(),
            claim_id: "claim-1".to_string(),
            execution_mode: "no_op".to_string(),
            local_node_id: "fog-01".to_string(),
            preflight_status: "warning".to_string(),
            checks: vec![],
            message: "preflight_status=warning checks=4 warnings=1 failures=0 local_node_id=fog-01 execution_mode=no_op safe_to_auto_execute=false side_effects_performed=false".to_string(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        };
        let rendered = format_submitted_assisted_execution_attempt_with_preflight(
            &preflight,
            &AssistedExecutionAttempt {
                task_id: Some("task-1".to_string()),
                request_id: Some("request-1".to_string()),
                claim_id: Some("claim-1".to_string()),
                attempt_id: Some("attempt-1".to_string()),
                attempt_status: Some("recorded".to_string()),
                execution_mode: Some("no_op".to_string()),
                execution_result: Some("no_op_recorded".to_string()),
                safe_to_auto_execute: Some(false),
                side_effects_performed: Some(false),
                ..AssistedExecutionAttempt::default()
            },
        );

        assert!(rendered.contains("Assisted execution preflight"));
        assert!(rendered.contains("Submitted assisted execution attempt"));
        assert!(rendered.contains(
            "Note: this client command performed preflight/no-op only, recorded attempt metadata on the server, and did not execute installation."
        ));
    }

    #[test]
    fn attempt_json_preserves_raw_response() {
        let response = AssistedExecutionAttempt {
            task_id: Some("task-1".to_string()),
            request_id: Some("request-1".to_string()),
            claim_id: Some("claim-1".to_string()),
            attempt_id: Some("attempt-1".to_string()),
            attempt_status: Some("recorded".to_string()),
            execution_mode: Some("no_op".to_string()),
            execution_result: Some("no_op_recorded".to_string()),
            safe_to_auto_execute: Some(false),
            side_effects_performed: Some(false),
            ..AssistedExecutionAttempt::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"task_id\": \"task-1\""));
        assert!(rendered.contains("\"request_id\": \"request-1\""));
        assert!(rendered.contains("\"claim_id\": \"claim-1\""));
        assert!(rendered.contains("\"attempt_id\": \"attempt-1\""));
        assert!(rendered.contains("\"attempt_status\": \"recorded\""));
        assert!(rendered.contains("\"execution_mode\": \"no_op\""));
        assert!(rendered.contains("\"execution_result\": \"no_op_recorded\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
        assert!(rendered.contains("\"side_effects_performed\": false"));
    }

    #[test]
    fn assisted_execution_state_formatting_includes_key_fields() {
        let rendered = format_assisted_execution_state_list(
            &AssistedExecutionStateSummaryList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                summaries: vec![AssistedExecutionStateSummary {
                    task_id: Some("task-1".to_string()),
                    state: Some("awaiting_approval".to_string()),
                    task_status: Some("planned".to_string()),
                    policy_decision: Some("candidate_for_assisted_execution".to_string()),
                    approval_status: Some("missing".to_string()),
                    readiness: Some("not_ready".to_string()),
                    request_status: Some("none".to_string()),
                    claim_status: Some("none".to_string()),
                    latest_execution_mode: Some("no_op".to_string()),
                    latest_execution_result: Some("no_op_recorded".to_string()),
                    safe_to_auto_execute: Some(false),
                    side_effects_performed: Some(false),
                    reason: Some("operator approval is required".to_string()),
                    ..AssistedExecutionStateSummary::default()
                }],
                ..AssistedExecutionStateSummaryList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("Assisted execution state for node fog-01"));
        assert!(rendered.contains("[AWAITING_APPROVAL]"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("task status: planned"));
        assert!(rendered.contains("policy: candidate_for_assisted_execution"));
        assert!(rendered.contains("approval: missing"));
        assert!(rendered.contains("readiness: not_ready"));
        assert!(rendered.contains("request: none"));
        assert!(rendered.contains("claim: none"));
        assert!(rendered.contains("latest mode: no_op"));
        assert!(rendered.contains("latest result: no_op_recorded"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("reason: operator approval is required"));
    }

    #[test]
    fn single_assisted_execution_state_includes_no_execution_note_for_attempts() {
        let rendered = format_single_assisted_execution_state(
            &AssistedExecutionStateSummary {
                task_id: Some("task-1".to_string()),
                state: Some("attempt_recorded_no_op".to_string()),
                task_status: Some("acknowledged".to_string()),
                policy_decision: Some("candidate_for_assisted_execution".to_string()),
                approval_status: Some("approved".to_string()),
                readiness: Some("ready_for_assisted_execution".to_string()),
                request_status: Some("requested".to_string()),
                claim_status: Some("claimed".to_string()),
                latest_attempt_status: Some("recorded".to_string()),
                latest_execution_mode: Some("no_op".to_string()),
                latest_execution_result: Some("no_op_recorded".to_string()),
                safe_to_auto_execute: Some(false),
                side_effects_performed: Some(false),
                reason: Some("latest no-op attempt was recorded".to_string()),
                ..AssistedExecutionStateSummary::default()
            },
            "task-1",
        );

        assert!(rendered.contains("Assisted execution state for task task-1"));
        assert!(rendered.contains("state: attempt_recorded_no_op"));
        assert!(rendered.contains("latest attempt: recorded"));
        assert!(rendered.contains("latest mode: no_op"));
        assert!(rendered.contains("latest result: no_op_recorded"));
        assert!(rendered.contains(
            "Note: this state is informational. No installation execution is performed by this command."
        ));
    }

    #[test]
    fn assisted_execution_state_flag_detects_attempt_states() {
        assert!(assisted_execution_state_has_attempt(
            &AssistedExecutionStateSummary {
                state: Some("attempt_recorded_dry_run".to_string()),
                ..AssistedExecutionStateSummary::default()
            }
        ));
        assert!(!assisted_execution_state_has_attempt(
            &AssistedExecutionStateSummary {
                state: Some("awaiting_request".to_string()),
                ..AssistedExecutionStateSummary::default()
            }
        ));
    }

    #[test]
    fn assisted_execution_state_json_preserves_raw_response() {
        let response = AssistedExecutionStateSummary {
            task_id: Some("task-1".to_string()),
            state: Some("attempt_recorded_no_op".to_string()),
            safe_to_auto_execute: Some(false),
            side_effects_performed: Some(false),
            ..AssistedExecutionStateSummary::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"task_id\": \"task-1\""));
        assert!(rendered.contains("\"state\": \"attempt_recorded_no_op\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
        assert!(rendered.contains("\"side_effects_performed\": false"));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        let result = validate_command_mode(
            Some("task-1"),
            None,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            true,
            true,
            false,
            None,
            false,
            Some("operator"),
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_accepts_approval_mode() {
        let result = validate_command_mode(
            Some("task-1"),
            None,
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            false,
            true,
            None,
            false,
            Some("operator"),
            None,
            None,
            None,
            None,
            Some("2026-06-01T00:00:00Z"),
            None,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn cli_mode_validation_rejects_readiness_with_approval_modes() {
        let result = validate_command_mode(
            Some("task-1"),
            None,
            None,
            false,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            true,
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_execution_request_ambiguity() {
        let result = validate_command_mode(
            Some("task-1"),
            None,
            None,
            false,
            false,
            false,
            true,
            true,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            None,
            Some("operator"),
            None,
            None,
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_claim_ambiguity() {
        let result = validate_command_mode(
            Some("task-1"),
            Some("request-1"),
            None,
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            true,
            None,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            None,
            None,
            Some("operator"),
            None,
            None,
            Some("2026-06-01T00:00:00Z"),
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_claim_status_ambiguity() {
        let result = validate_command_mode(
            Some("task-1"),
            Some("request-1"),
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            true,
            false,
            Some("released"),
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_attempt_ambiguity() {
        let result = validate_command_mode(
            Some("task-1"),
            Some("request-1"),
            Some("claim-1"),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            true,
            true,
            Some("no_op"),
            false,
            false,
            false,
            None,
            false,
            None,
            None,
            None,
            Some("operator"),
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_execution_state_ambiguity() {
        let result = validate_command_mode(
            Some("task-1"),
            None,
            None,
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_accepts_execution_state_for_single_task() {
        let result = validate_command_mode(
            Some("task-1"),
            None,
            None,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            false,
            false,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );

        assert!(result.is_ok());
    }
}
