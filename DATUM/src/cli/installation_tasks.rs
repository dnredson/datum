use anyhow::{anyhow, Context, Result};
use std::path::Path;

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
            fetch_local_mapping_patch_proposals_for_node,
            fetch_local_mapping_recommendations_for_node, materialize_installation_tasks_for_node,
            patch_assisted_execution_claim_status, patch_installation_task_status,
            publish_local_mapping_preflight_report, submit_assisted_execution_attempt,
            submit_assisted_execution_claim, submit_assisted_execution_request,
            submit_installation_task_approval,
        },
        model::{
            AssistedExecutionAttempt, AssistedExecutionAttemptList, AssistedExecutionClaim,
            AssistedExecutionClaimList, AssistedExecutionRequest, AssistedExecutionRequestList,
            AssistedExecutionStateSummary, AssistedExecutionStateSummaryList, InstallationTask,
            InstallationTaskApproval, InstallationTaskApprovalList, InstallationTaskList,
            InstallationTaskReadiness, InstallationTaskReadinessList, LocalMappingPatchProposal,
            LocalMappingPatchProposalList, LocalMappingPatchRevalidationReport,
            LocalMappingRecommendation, LocalMappingRecommendationList,
        },
    },
    local_mapping::{
        patch_file::{
            generate_local_mapping_patch_file_content, write_local_mapping_patch_proposal_file,
        },
        patch_revalidator::revalidate_local_mapping_patch_file,
        preflight::{
            run_local_mapping_preflight, LocalMappingPreflightItem, LocalMappingPreflightReport,
        },
    },
};

const ALLOWED_TASK_STATUSES: &[&str] = &["acknowledged", "blocked", "cancelled", "completed"];
const ALLOWED_CLAIM_STATUSES: &[&str] =
    &["claimed", "released", "expired", "completed", "cancelled"];
const ALLOWED_ATTEMPT_EXECUTION_MODES: &[&str] = &["no_op", "dry_run"];

pub fn run_installation_tasks_command(
    cfg: &Config,
    config_path: &str,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    task_id: Option<&str>,
    request_id: Option<&str>,
    claim_id: Option<&str>,
    local_mapping_preflight: bool,
    publish_local_mapping_preflight: bool,
    local_mapping_recommendations: bool,
    local_mapping_patch_proposals: bool,
    write_local_mapping_patch: Option<&str>,
    overwrite_local_mapping_patch: bool,
    revalidate_local_mapping_patch: Option<&str>,
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
        local_mapping_preflight,
        publish_local_mapping_preflight,
        local_mapping_recommendations,
        local_mapping_patch_proposals,
        write_local_mapping_patch,
        overwrite_local_mapping_patch,
        revalidate_local_mapping_patch,
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

    if let Some(proposal_path) = revalidate_local_mapping_patch {
        let report = revalidate_local_mapping_patch_file(
            cfg,
            Path::new(config_path),
            Path::new(proposal_path),
        )?;

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&report)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!(
            "{}",
            format_local_mapping_patch_revalidation_report(&report)
        );
        return Ok(());
    }

    if local_mapping_preflight {
        if let Some(task_id) = task_id {
            let task = fetch_installation_task(&base_url, &project_id, &node_id, task_id)?;
            let response = run_local_mapping_preflight(cfg, &node_id, &[task]);

            if publish_local_mapping_preflight {
                assert_local_mapping_preflight_publish_invariants(&response)?;
                let server_report = publish_local_mapping_preflight_report(
                    &base_url,
                    &project_id,
                    &node_id,
                    &response,
                )?;

                if json_output {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "local_report": response,
                            "server_report": server_report
                        }))
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                    );
                    return Ok(());
                }

                print!(
                    "{}{}",
                    format_local_mapping_preflight_report(&response),
                    format_published_local_mapping_preflight_report(&server_report)
                );
                return Ok(());
            }

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&response)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_local_mapping_preflight_report(&response));
            return Ok(());
        }

        let response = fetch_installation_tasks_for_node(&base_url, &project_id, &node_id)?;
        let report = run_local_mapping_preflight(cfg, &node_id, &response.tasks);

        if publish_local_mapping_preflight {
            assert_local_mapping_preflight_publish_invariants(&report)?;
            let server_report =
                publish_local_mapping_preflight_report(&base_url, &project_id, &node_id, &report)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "local_report": report,
                        "server_report": server_report
                    }))
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!(
                "{}{}",
                format_local_mapping_preflight_report(&report),
                format_published_local_mapping_preflight_report(&server_report)
            );
            return Ok(());
        }

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&report)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", format_local_mapping_preflight_report(&report));
        return Ok(());
    }

    if local_mapping_recommendations {
        let response =
            fetch_local_mapping_recommendations_for_node(&base_url, &project_id, &node_id)?;

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
            format_local_mapping_recommendation_list(&response, &project_id, &node_id)
        );
        return Ok(());
    }

    if local_mapping_patch_proposals {
        let response =
            fetch_local_mapping_patch_proposals_for_node(&base_url, &project_id, &node_id)?;
        let rendered = format_local_mapping_patch_proposal_list(&response, &project_id, &node_id);

        if let Some(output_path) = write_local_mapping_patch {
            let output_path = Path::new(output_path);
            let content = generate_local_mapping_patch_file_content(
                &response.proposals,
                &node_id,
                &project_id,
            );
            write_local_mapping_patch_proposal_file(
                output_path,
                &content,
                overwrite_local_mapping_patch,
            )
            .with_context(|| {
                format!(
                    "write local mapping patch proposal file to {}",
                    output_path.display()
                )
            })?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "proposals": response.proposals,
                        "output_path": output_path.display().to_string(),
                        "wrote_file": true,
                        "applied": false,
                        "advisory": true
                    }))
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            println!("{}", rendered);
            println!();
            println!("Wrote local mapping patch proposal file");
            println!("  path: {}", output_path.display());
            println!("  advisory: true");
            println!("  applied: false");
            println!("  safe to auto apply: false");
            return Ok(());
        }

        if json_output {
            println!(
                "{}",
                serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
            );
            return Ok(());
        }

        print!("{}", rendered);
        return Ok(());
    }

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
    local_mapping_preflight: bool,
    publish_local_mapping_preflight: bool,
    local_mapping_recommendations: bool,
    local_mapping_patch_proposals: bool,
    write_local_mapping_patch: Option<&str>,
    overwrite_local_mapping_patch: bool,
    revalidate_local_mapping_patch: Option<&str>,
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
        local_mapping_preflight && task_id.is_none(),
        local_mapping_preflight && task_id.is_some(),
        local_mapping_recommendations,
        local_mapping_patch_proposals,
        revalidate_local_mapping_patch.is_some(),
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
            && !local_mapping_preflight
            && !local_mapping_recommendations
            && !local_mapping_patch_proposals
            && revalidate_local_mapping_patch.is_none()
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
            "ambiguous installation-tasks mode; choose only one of default list, --materialize, --task-id, --status, --local-mapping-preflight, --local-mapping-recommendations, --local-mapping-patch-proposals, --readiness, --execution-request, --execution-requests, --request-assisted-execution, --execution-claim, --execution-claims, --claim-assisted-execution, --claim-status, --execution-attempt, --execution-attempts, --attempt-assisted-execution, --approval, --approvals, or --approve"
        ));
    }

    if local_mapping_preflight && materialize {
        return Err(anyhow!(
            "--local-mapping-preflight cannot be combined with --materialize; run --materialize first if needed"
        ));
    }

    if local_mapping_preflight && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --local-mapping-preflight"
        ));
    }

    if local_mapping_preflight && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --local-mapping-preflight"
        ));
    }

    if publish_local_mapping_preflight && !local_mapping_preflight {
        return Err(anyhow!(
            "--publish-local-mapping-preflight requires --local-mapping-preflight"
        ));
    }

    if local_mapping_recommendations && task_id.is_some() {
        return Err(anyhow!(
            "--task-id cannot be combined with --local-mapping-recommendations"
        ));
    }

    if local_mapping_recommendations && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --local-mapping-recommendations"
        ));
    }

    if local_mapping_recommendations && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --local-mapping-recommendations"
        ));
    }

    if local_mapping_recommendations && materialize {
        return Err(anyhow!(
            "--local-mapping-recommendations cannot be combined with --materialize"
        ));
    }

    if local_mapping_patch_proposals && task_id.is_some() {
        return Err(anyhow!(
            "--task-id cannot be combined with --local-mapping-patch-proposals"
        ));
    }

    if local_mapping_patch_proposals && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --local-mapping-patch-proposals"
        ));
    }

    if local_mapping_patch_proposals && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --local-mapping-patch-proposals"
        ));
    }

    if local_mapping_patch_proposals && materialize {
        return Err(anyhow!(
            "--local-mapping-patch-proposals cannot be combined with --materialize"
        ));
    }

    if write_local_mapping_patch.is_some() && !local_mapping_patch_proposals {
        return Err(anyhow!(
            "--write-local-mapping-patch requires --local-mapping-patch-proposals"
        ));
    }

    if overwrite_local_mapping_patch && write_local_mapping_patch.is_none() {
        return Err(anyhow!(
            "--overwrite-local-mapping-patch requires --write-local-mapping-patch"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && local_mapping_patch_proposals {
        return Err(anyhow!(
            "--revalidate-local-mapping-patch cannot be combined with --local-mapping-patch-proposals"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && write_local_mapping_patch.is_some() {
        return Err(anyhow!(
            "--revalidate-local-mapping-patch cannot be combined with --write-local-mapping-patch"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && overwrite_local_mapping_patch {
        return Err(anyhow!(
            "--revalidate-local-mapping-patch cannot be combined with --overwrite-local-mapping-patch"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && publish_local_mapping_preflight {
        return Err(anyhow!(
            "--revalidate-local-mapping-patch cannot be combined with --publish-local-mapping-preflight"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && task_id.is_some() {
        return Err(anyhow!(
            "--task-id cannot be combined with --revalidate-local-mapping-patch"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && request_id.is_some() {
        return Err(anyhow!(
            "--request-id cannot be combined with --revalidate-local-mapping-patch"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && claim_id.is_some() {
        return Err(anyhow!(
            "--claim-id cannot be combined with --revalidate-local-mapping-patch"
        ));
    }

    if revalidate_local_mapping_patch.is_some() && materialize {
        return Err(anyhow!(
            "--revalidate-local-mapping-patch cannot be combined with --materialize"
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

    if response
        .summaries
        .iter()
        .any(assisted_execution_state_has_local_mapping_advisory_note)
    {
        lines.push(String::new());
        lines.push(
            "Note: local mapping context is advisory only. This command did not write config and did not execute installation."
                .to_string(),
        );
    }

    lines.join("\n")
}

fn format_local_mapping_preflight_report(report: &LocalMappingPreflightReport) -> String {
    let mut lines = vec![
        format!("Local mapping preflight for node {}", report.node_id),
        format!("Status: {}", report.status),
    ];

    if report.status == "empty" {
        lines.push("No installation tasks require local mapping preflight.".to_string());
        lines.push(format!(
            "Safe to auto execute: {}",
            report.safe_to_auto_execute
        ));
        lines.push(format!(
            "Side effects performed: {}",
            report.side_effects_performed
        ));
        lines.push(String::new());
        lines.push(
            "Note: this preflight is advisory only. No local config was written and no installation was executed."
                .to_string(),
        );
        return lines.join("\n");
    }

    lines.push(format!("Items: {}", report.summary.applicable_items));
    lines.push(format!(
        "Already mapped: {} | Missing: {} | Ambiguous: {} | Not applicable: {}",
        report.summary.already_mapped,
        report.summary.missing_mapping,
        report.summary.ambiguous,
        report.summary.not_applicable
    ));
    lines.push(format!(
        "Safe to auto execute: {}",
        report.safe_to_auto_execute
    ));
    lines.push(format!(
        "Side effects performed: {}",
        report.side_effects_performed
    ));

    for item in report
        .items
        .iter()
        .filter(|item| item.status != "not_applicable")
    {
        lines.push(String::new());
        lines.extend(format_local_mapping_preflight_item(item));
    }

    lines.push(String::new());
    lines.push(
        "Note: this preflight is advisory only. No local config was written and no installation was executed."
            .to_string(),
    );
    lines.join("\n")
}

fn format_published_local_mapping_preflight_report(report: &LocalMappingPreflightReport) -> String {
    let mut lines = vec![
        String::new(),
        "Published local mapping preflight report".to_string(),
    ];

    if let Some(report_id) = report.report_id.as_deref() {
        lines.push(format!("  report_id: {}", report_id));
    }
    if let Some(reported_at_utc) = report.reported_at_utc.as_deref() {
        lines.push(format!("  reported_at_utc: {}", reported_at_utc));
    }
    lines.push(format!("  status: {}", report.status));
    lines.push(format!(
        "  safe to auto execute: {}",
        report.safe_to_auto_execute
    ));
    lines.push(format!(
        "  side effects performed: {}",
        report.side_effects_performed
    ));
    lines.push(String::new());
    lines.push(
        "Note: this command did not write local config and did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn assert_local_mapping_preflight_publish_invariants(
    report: &LocalMappingPreflightReport,
) -> Result<()> {
    if report.safe_to_auto_execute {
        return Err(anyhow!(
            "local mapping preflight publish rejected because safe_to_auto_execute must remain false"
        ));
    }

    if report.side_effects_performed {
        return Err(anyhow!(
            "local mapping preflight publish rejected because side_effects_performed must remain false"
        ));
    }

    Ok(())
}

fn format_local_mapping_recommendation_list(
    response: &LocalMappingRecommendationList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response
        .count
        .unwrap_or(response.recommendations.len() as u64);

    let mut lines = vec![
        format!(
            "Local mapping recommendations for node {}",
            effective_node_id
        ),
        format!("Project: {}", effective_project_id),
    ];

    if response.recommendations.is_empty() {
        lines.push("No local mapping recommendations for this node.".to_string());
        lines.push(String::new());
        lines.push(
            "Local mapping recommendations are advisory only. This command did not write config and did not execute installation."
                .to_string(),
        );
        return lines.join("\n");
    }

    lines.push(format!("Recommendations: {}", count));
    lines.push(String::new());

    for (index, recommendation) in response.recommendations.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_local_mapping_recommendation(recommendation));
    }

    lines.push(String::new());
    lines.push(
        "Local mapping recommendations are advisory only. This command did not write config and did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn format_local_mapping_recommendation(recommendation: &LocalMappingRecommendation) -> Vec<String> {
    let status = recommendation
        .recommendation_status
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let priority = recommendation
        .priority
        .as_deref()
        .map(|value| format!("[{}]", value.to_ascii_uppercase()))
        .unwrap_or_default();
    let title = recommendation
        .module_id
        .as_deref()
        .or(recommendation.suggested_mapping_key.as_deref())
        .or(recommendation.task_id.as_deref())
        .unwrap_or("unknown");

    let mut lines = vec![format!("[{}]{} {}", status, priority, title)];

    if let Some(recommendation_id) = recommendation.recommendation_id.as_deref() {
        lines.push(format!("  recommendation_id: {}", recommendation_id));
    }
    if let Some(task_id) = recommendation.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }
    if let Some(module_id) = recommendation.module_id.as_deref() {
        lines.push(format!("  module_id: {}", module_id));
    }
    if let Some(runtime) = recommendation.runtime.as_deref() {
        lines.push(format!("  runtime: {}", runtime));
    }
    if let Some(artifact) = recommendation.artifact.as_deref() {
        lines.push(format!("  artifact: {}", artifact));
    }
    if let Some(mapping_key) = recommendation.suggested_mapping_key.as_deref() {
        lines.push(format!("  suggested mapping key: {}", mapping_key));
    }
    if let Some(container_name) = recommendation.suggested_container_name.as_deref() {
        lines.push(format!("  suggested container: {}", container_name));
    }
    if let Some(suggested_artifact) = recommendation.suggested_artifact.as_deref() {
        lines.push(format!("  suggested artifact: {}", suggested_artifact));
    }

    lines.push(format!(
        "  safe to auto apply: {}",
        recommendation.safe_to_auto_apply.unwrap_or(false)
    ));
    lines.push(format!(
        "  requires operator: {}",
        recommendation.requires_operator.unwrap_or(true)
    ));
    lines.push(format!(
        "  reason: {}",
        recommendation.reason.as_deref().unwrap_or("unknown")
    ));

    lines
}

fn format_local_mapping_patch_proposal_list(
    response: &LocalMappingPatchProposalList,
    project_id: &str,
    requested_node_id: &str,
) -> String {
    let effective_node_id = response.node_id.as_deref().unwrap_or(requested_node_id);
    let effective_project_id = response.project_id.as_deref().unwrap_or(project_id);
    let count = response.count.unwrap_or(response.proposals.len() as u64);

    let mut lines = vec![
        format!(
            "Local mapping patch proposals for node {}",
            effective_node_id
        ),
        format!("Project: {}", effective_project_id),
    ];

    if response.proposals.is_empty() {
        lines.push("No local mapping patch proposals for this node.".to_string());
        lines.push(String::new());
        lines.push(
            "Note: this proposal viewer is advisory only. This command did not write config, did not apply mappings, and did not execute installation."
                .to_string(),
        );
        return lines.join("\n");
    }

    lines.push(format!("Proposals: {}", count));
    lines.push(String::new());

    for (index, proposal) in response.proposals.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(format_local_mapping_patch_proposal(proposal));
    }

    lines.push(String::new());
    lines.push(
        "Note: this proposal viewer is advisory only. This command did not write config, did not apply mappings, and did not execute installation."
            .to_string(),
    );

    lines.join("\n")
}

fn format_local_mapping_patch_proposal(proposal: &LocalMappingPatchProposal) -> Vec<String> {
    let status = proposal
        .proposal_status
        .as_deref()
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let kind = proposal.proposal_kind.as_deref().unwrap_or("unknown");
    let mut lines = vec![format!("[{}] {}", status, kind)];

    if let Some(proposal_id) = proposal.proposal_id.as_deref() {
        lines.push(format!("  proposal_id: {}", proposal_id));
    }
    if let Some(task_id) = proposal.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }
    if let Some(module_id) = proposal.module_id.as_deref() {
        lines.push(format!("  module: {}", module_id));
    }
    if let Some(runtime) = proposal.runtime.as_deref() {
        lines.push(format!("  runtime: {}", runtime));
    }
    if let Some(artifact) = proposal.artifact.as_deref() {
        lines.push(format!("  artifact: {}", artifact));
    }
    if let Some(mapping_key) = proposal.suggested_mapping_key.as_deref() {
        lines.push(format!("  suggested mapping key: {}", mapping_key));
    }
    if let Some(container_name) = proposal.suggested_container_name.as_deref() {
        lines.push(format!("  suggested container name: {}", container_name));
    } else if proposal.proposed_config_snippet.is_some() {
        lines.push("  suggested container name: REVIEW_AND_SET_CONTAINER_NAME".to_string());
    }
    if let Some(config_section) = proposal.proposed_config_section.as_deref() {
        lines.push(format!("  config section: {}", config_section));
    }

    lines.push(format!(
        "  safe to auto apply: {}",
        proposal.safe_to_auto_apply.unwrap_or(false)
    ));
    lines.push(format!(
        "  side effects performed: {}",
        proposal.side_effects_performed.unwrap_or(false)
    ));
    lines.push(format!(
        "  requires operator: {}",
        proposal.requires_operator.unwrap_or(true)
    ));
    lines.push(format!(
        "  reason: {}",
        proposal.reason.as_deref().unwrap_or("unknown")
    ));

    if let Some(snippet) = proposal.proposed_config_snippet.as_deref() {
        lines.push(String::new());
        lines.push("Suggested config snippet:".to_string());
        for line in snippet.lines() {
            lines.push(format!("  {}", line));
        }
    }

    lines
}

fn format_local_mapping_patch_revalidation_report(
    report: &LocalMappingPatchRevalidationReport,
) -> String {
    let mut lines = vec![
        "Local mapping patch revalidation".to_string(),
        format!("Config: {}", report.config_path),
        format!("Proposal: {}", report.proposal_path),
        format!("Status: {}", report.status),
        format!("Items: {}", report.summary.total_items),
        format!(
            "Valid new: {} | Already present: {} | Conflicts: {} | Incomplete: {} | Ignored: {} | Warnings: {}",
            report.summary.valid_new_mappings,
            report.summary.already_present,
            report.summary.conflicts,
            report.summary.incomplete,
            report.summary.ignored,
            report.summary.warnings
        ),
        format!("Safe to auto apply: {}", report.safe_to_auto_apply),
        format!(
            "Side effects performed: {}",
            report.side_effects_performed
        ),
        format!("Applied: {}", report.applied),
    ];

    if report.items.is_empty() {
        lines.push(String::new());
        lines.push("No mapping entries were found in the proposal file.".to_string());
    } else {
        for item in &report.items {
            lines.push(String::new());
            lines.push(format!(
                "[{}] {}",
                item.status.to_ascii_uppercase(),
                item.mapping_key
                    .as_deref()
                    .unwrap_or("(unknown mapping key)")
            ));
            if let Some(proposed) = item.proposed_container_name.as_deref() {
                lines.push(format!("  proposed container: {}", proposed));
            }
            if let Some(current) = item.current_container_name.as_deref() {
                lines.push(format!("  current container: {}", current));
            }
            lines.push(format!("  reason: {}", item.reason));
            if let Some(source_line) = item.source_line.as_deref() {
                lines.push(format!("  source line: {}", source_line));
            }
        }
    }

    lines.push(String::new());
    lines.push(
        "Note: this command did not modify config.toml, did not apply mappings, and did not execute installation."
            .to_string(),
    );

    lines.join("\n") + "\n"
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

    if assisted_execution_state_has_local_mapping_advisory_note(summary) {
        lines.push(String::new());
        lines.push(
            "Note: local mapping context is advisory only. This command did not write config and did not execute installation."
                .to_string(),
        );
    }

    lines.join("\n")
}

fn format_local_mapping_preflight_item(item: &LocalMappingPreflightItem) -> Vec<String> {
    let label = item.status.to_ascii_uppercase();
    let title = item
        .module_id
        .as_deref()
        .or(item.artifact.as_deref())
        .or(item.action_id.as_deref())
        .unwrap_or("unknown");
    let mut lines = vec![format!("[{}] {}", label, title)];

    if let Some(task_id) = item.task_id.as_deref() {
        lines.push(format!("  task_id: {}", task_id));
    }
    if let Some(runtime) = item.runtime.as_deref() {
        lines.push(format!("  runtime: {}", runtime));
    }
    if let Some(artifact) = item.artifact.as_deref() {
        lines.push(format!("  artifact: {}", artifact));
    }
    if let Some(source) = item.current_mapping_source.as_deref() {
        lines.push(format!("  mapping source: {}", source));
    }
    if let Some(container) = item.current_container_name.as_deref() {
        lines.push(format!("  container: {}", container));
    }
    if let Some(key) = item.suggested_mapping_key.as_deref() {
        lines.push(format!("  suggested mapping key: {}", key));
    }
    if let Some(artifact) = item.suggested_artifact.as_deref() {
        lines.push(format!("  suggested artifact: {}", artifact));
    }
    lines.push(format!("  reason: {}", item.reason));
    lines
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

    if let Some(local_mapping_status) = summary.local_mapping_status.as_deref() {
        lines.push(format!("  local mapping: {}", local_mapping_status));
    }

    if let Some(local_mapping_reason) = summary.local_mapping_reason.as_deref() {
        lines.push(format!("  local mapping reason: {}", local_mapping_reason));
    }

    if let Some(local_mapping_report_id) = summary.local_mapping_report_id.as_deref() {
        lines.push(format!(
            "  local mapping report: {}",
            local_mapping_report_id
        ));
    }

    if let Some(local_mapping_reported_at_utc) = summary.local_mapping_reported_at_utc.as_deref() {
        lines.push(format!(
            "  local mapping reported at: {}",
            local_mapping_reported_at_utc
        ));
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

fn assisted_execution_state_has_local_mapping_advisory_note(
    summary: &AssistedExecutionStateSummary,
) -> bool {
    matches!(
        summary.local_mapping_status.as_deref(),
        Some("missing_mapping") | Some("ambiguous")
    )
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
        assert_local_mapping_preflight_publish_invariants, assisted_execution_state_has_attempt,
        assisted_execution_state_has_local_mapping_advisory_note,
        build_attempt_message_with_preflight, format_approval_list,
        format_assisted_execution_attempt_list, format_assisted_execution_claim_list,
        format_assisted_execution_preflight_report, format_assisted_execution_request_list,
        format_assisted_execution_state_list, format_latest_approval,
        format_latest_assisted_execution_attempt, format_latest_assisted_execution_claim,
        format_latest_assisted_execution_request, format_local_mapping_patch_proposal_list,
        format_local_mapping_patch_revalidation_report, format_local_mapping_preflight_report,
        format_local_mapping_recommendation_list, format_published_local_mapping_preflight_report,
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
            LocalMappingPatchProposal, LocalMappingPatchProposalList,
            LocalMappingPatchRevalidationItem, LocalMappingPatchRevalidationReport,
            LocalMappingPatchRevalidationSummary, LocalMappingRecommendation,
            LocalMappingRecommendationList,
        },
        local_mapping::preflight::{
            LocalMappingPreflightItem, LocalMappingPreflightReport, LocalMappingPreflightSummary,
        },
    };

    #[derive(Default)]
    struct ModeCase<'a> {
        task_id: Option<&'a str>,
        request_id: Option<&'a str>,
        claim_id: Option<&'a str>,
        local_mapping_preflight: bool,
        local_mapping_recommendations: bool,
        local_mapping_patch_proposals: bool,
        write_local_mapping_patch: Option<&'a str>,
        overwrite_local_mapping_patch: bool,
        revalidate_local_mapping_patch: Option<&'a str>,
        execution_state: bool,
        readiness: bool,
        execution_requests: bool,
        execution_request: bool,
        request_assisted_execution: bool,
        execution_claims: bool,
        execution_claim: bool,
        claim_assisted_execution: bool,
        claim_status: Option<&'a str>,
        execution_attempts: bool,
        execution_attempt: bool,
        attempt_assisted_execution: bool,
        execution_mode: Option<&'a str>,
        approvals: bool,
        approval: bool,
        approve: bool,
        status: Option<&'a str>,
        materialize: bool,
        approved_by: Option<&'a str>,
        requested_by: Option<&'a str>,
        claimed_by: Option<&'a str>,
        attempted_by: Option<&'a str>,
        message: Option<&'a str>,
        expires_at_utc: Option<&'a str>,
        lease_expires_at_utc: Option<&'a str>,
    }

    fn validate_mode_case(case: ModeCase<'_>) -> anyhow::Result<()> {
        validate_command_mode(
            case.task_id,
            case.request_id,
            case.claim_id,
            case.local_mapping_preflight,
            false,
            case.local_mapping_recommendations,
            case.local_mapping_patch_proposals,
            case.write_local_mapping_patch,
            case.overwrite_local_mapping_patch,
            case.revalidate_local_mapping_patch,
            case.execution_state,
            case.readiness,
            case.execution_requests,
            case.execution_request,
            case.request_assisted_execution,
            case.execution_claims,
            case.execution_claim,
            case.claim_assisted_execution,
            case.claim_status,
            case.execution_attempts,
            case.execution_attempt,
            case.attempt_assisted_execution,
            case.execution_mode,
            case.approvals,
            case.approval,
            case.approve,
            case.status,
            case.materialize,
            case.approved_by,
            case.requested_by,
            case.claimed_by,
            case.attempted_by,
            case.message,
            case.expires_at_utc,
            case.lease_expires_at_utc,
        )
    }

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
    fn assisted_execution_state_formatting_includes_local_mapping_fields() {
        let rendered = format_assisted_execution_state_list(
            &AssistedExecutionStateSummaryList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                summaries: vec![AssistedExecutionStateSummary {
                    task_id: Some("task-1".to_string()),
                    state: Some("awaiting_request".to_string()),
                    local_mapping_status: Some("missing_mapping".to_string()),
                    local_mapping_reason: Some(
                        "no trusted local mapping found in DATUM config".to_string(),
                    ),
                    local_mapping_report_id: Some("report-1".to_string()),
                    local_mapping_reported_at_utc: Some("2026-05-20T12:00:00Z".to_string()),
                    safe_to_auto_execute: Some(false),
                    side_effects_performed: Some(false),
                    ..AssistedExecutionStateSummary::default()
                }],
                ..AssistedExecutionStateSummaryList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("local mapping: missing_mapping"));
        assert!(rendered
            .contains("local mapping reason: no trusted local mapping found in DATUM config"));
        assert!(rendered.contains("local mapping report: report-1"));
        assert!(rendered.contains("local mapping reported at: 2026-05-20T12:00:00Z"));
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
    fn single_assisted_execution_state_missing_mapping_includes_advisory_note() {
        let rendered = format_single_assisted_execution_state(
            &AssistedExecutionStateSummary {
                task_id: Some("task-1".to_string()),
                state: Some("awaiting_request".to_string()),
                local_mapping_status: Some("missing_mapping".to_string()),
                local_mapping_reason: Some(
                    "no trusted local mapping found in DATUM config".to_string(),
                ),
                safe_to_auto_execute: Some(false),
                side_effects_performed: Some(false),
                ..AssistedExecutionStateSummary::default()
            },
            "task-1",
        );

        assert!(rendered.contains("local mapping: missing_mapping"));
        assert!(rendered.contains(
            "Note: local mapping context is advisory only. This command did not write config and did not execute installation."
        ));
    }

    #[test]
    fn single_assisted_execution_state_ambiguous_includes_advisory_note() {
        let rendered = format_single_assisted_execution_state(
            &AssistedExecutionStateSummary {
                task_id: Some("task-1".to_string()),
                state: Some("awaiting_request".to_string()),
                local_mapping_status: Some("ambiguous".to_string()),
                local_mapping_reason: Some("multiple local mapping candidates".to_string()),
                safe_to_auto_execute: Some(false),
                side_effects_performed: Some(false),
                ..AssistedExecutionStateSummary::default()
            },
            "task-1",
        );

        assert!(rendered.contains("local mapping: ambiguous"));
        assert!(rendered.contains(
            "Note: local mapping context is advisory only. This command did not write config and did not execute installation."
        ));
    }

    #[test]
    fn single_assisted_execution_state_already_mapped_does_not_imply_execution_permission() {
        let rendered = format_single_assisted_execution_state(
            &AssistedExecutionStateSummary {
                task_id: Some("task-1".to_string()),
                state: Some("awaiting_request".to_string()),
                local_mapping_status: Some("already_mapped".to_string()),
                local_mapping_reason: Some("trusted mapping found".to_string()),
                safe_to_auto_execute: Some(false),
                side_effects_performed: Some(false),
                ..AssistedExecutionStateSummary::default()
            },
            "task-1",
        );

        assert!(rendered.contains("local mapping: already_mapped"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(!rendered.contains("execution is allowed"));
        assert!(!rendered.contains(
            "Note: local mapping context is advisory only. This command did not write config and did not execute installation."
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
    fn assisted_execution_state_advisory_flag_detects_missing_or_ambiguous() {
        assert!(assisted_execution_state_has_local_mapping_advisory_note(
            &AssistedExecutionStateSummary {
                local_mapping_status: Some("missing_mapping".to_string()),
                ..AssistedExecutionStateSummary::default()
            }
        ));
        assert!(assisted_execution_state_has_local_mapping_advisory_note(
            &AssistedExecutionStateSummary {
                local_mapping_status: Some("ambiguous".to_string()),
                ..AssistedExecutionStateSummary::default()
            }
        ));
        assert!(!assisted_execution_state_has_local_mapping_advisory_note(
            &AssistedExecutionStateSummary {
                local_mapping_status: Some("already_mapped".to_string()),
                ..AssistedExecutionStateSummary::default()
            }
        ));
    }

    #[test]
    fn assisted_execution_state_json_preserves_raw_response() {
        let response = AssistedExecutionStateSummary {
            task_id: Some("task-1".to_string()),
            state: Some("attempt_recorded_no_op".to_string()),
            local_mapping_status: Some("unknown".to_string()),
            local_mapping_reason: Some(
                "no matching local mapping preflight item for task".to_string(),
            ),
            local_mapping_report_id: Some("report-1".to_string()),
            local_mapping_reported_at_utc: Some("2026-05-20T12:00:00Z".to_string()),
            local_mapping_item_snapshot: Some(serde_json::json!({"task_id":"task-1"})),
            local_mapping_preflight_snapshot: Some(serde_json::json!({"report_id":"report-1"})),
            safe_to_auto_execute: Some(false),
            side_effects_performed: Some(false),
            ..AssistedExecutionStateSummary::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"task_id\": \"task-1\""));
        assert!(rendered.contains("\"state\": \"attempt_recorded_no_op\""));
        assert!(rendered.contains("\"local_mapping_status\": \"unknown\""));
        assert!(rendered.contains(
            "\"local_mapping_reason\": \"no matching local mapping preflight item for task\""
        ));
        assert!(rendered.contains("\"local_mapping_report_id\": \"report-1\""));
        assert!(rendered.contains("\"local_mapping_reported_at_utc\": \"2026-05-20T12:00:00Z\""));
        assert!(rendered.contains("\"local_mapping_item_snapshot\""));
        assert!(rendered.contains("\"local_mapping_preflight_snapshot\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
        assert!(rendered.contains("\"side_effects_performed\": false"));
    }

    #[test]
    fn local_mapping_preflight_formatting_includes_missing_mapping_and_note() {
        let rendered = format_local_mapping_preflight_report(&LocalMappingPreflightReport {
            node_id: "fog-01".to_string(),
            status: "warning".to_string(),
            items: vec![LocalMappingPreflightItem {
                task_id: Some("task-1".to_string()),
                module_id: Some("lorawan-bridge".to_string()),
                runtime: Some("container".to_string()),
                artifact: Some("example/lorawan-bridge:latest".to_string()),
                status: "missing_mapping".to_string(),
                reason: "no trusted local mapping found in DATUM config".to_string(),
                suggested_mapping_key: Some("lorawan-bridge".to_string()),
                suggested_artifact: Some("example/lorawan-bridge:latest".to_string()),
                ..LocalMappingPreflightItem::default()
            }],
            summary: LocalMappingPreflightSummary {
                total_items: 1,
                applicable_items: 1,
                already_mapped: 0,
                missing_mapping: 1,
                ambiguous: 0,
                not_applicable: 0,
                warnings: 1,
            },
            safe_to_auto_execute: false,
            side_effects_performed: false,
            ..LocalMappingPreflightReport::default()
        });

        assert!(rendered.contains("Local mapping preflight for node fog-01"));
        assert!(rendered.contains("Status: warning"));
        assert!(rendered.contains("[MISSING_MAPPING] lorawan-bridge"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("runtime: container"));
        assert!(rendered.contains("artifact: example/lorawan-bridge:latest"));
        assert!(rendered.contains("suggested mapping key: lorawan-bridge"));
        assert!(rendered.contains("Safe to auto execute: false"));
        assert!(rendered.contains("Side effects performed: false"));
        assert!(rendered.contains(
            "Note: this preflight is advisory only. No local config was written and no installation was executed."
        ));
    }

    #[test]
    fn local_mapping_preflight_json_preserves_raw_response() {
        let response = LocalMappingPreflightReport {
            node_id: "fog-01".to_string(),
            status: "warning".to_string(),
            items: vec![],
            summary: LocalMappingPreflightSummary::default(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            ..LocalMappingPreflightReport::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"node_id\": \"fog-01\""));
        assert!(rendered.contains("\"status\": \"warning\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
        assert!(rendered.contains("\"side_effects_performed\": false"));
    }

    #[test]
    fn local_mapping_preflight_publish_invariant_rejects_safe_to_auto_execute_true() {
        let result =
            assert_local_mapping_preflight_publish_invariants(&LocalMappingPreflightReport {
                node_id: "fog-01".to_string(),
                status: "warning".to_string(),
                safe_to_auto_execute: true,
                side_effects_performed: false,
                ..LocalMappingPreflightReport::default()
            });

        assert!(result.is_err());
    }

    #[test]
    fn local_mapping_preflight_publish_invariant_rejects_side_effects_performed_true() {
        let result =
            assert_local_mapping_preflight_publish_invariants(&LocalMappingPreflightReport {
                node_id: "fog-01".to_string(),
                status: "warning".to_string(),
                safe_to_auto_execute: false,
                side_effects_performed: true,
                ..LocalMappingPreflightReport::default()
            });

        assert!(result.is_err());
    }

    #[test]
    fn publish_output_includes_report_id_status_and_safety_fields() {
        let rendered =
            format_published_local_mapping_preflight_report(&LocalMappingPreflightReport {
                node_id: "fog-01".to_string(),
                report_id: Some("report-1".to_string()),
                reported_at_utc: Some("2026-05-20T12:00:00Z".to_string()),
                status: "warning".to_string(),
                safe_to_auto_execute: false,
                side_effects_performed: false,
                ..LocalMappingPreflightReport::default()
            });

        assert!(rendered.contains("Published local mapping preflight report"));
        assert!(rendered.contains("report_id: report-1"));
        assert!(rendered.contains("reported_at_utc: 2026-05-20T12:00:00Z"));
        assert!(rendered.contains("status: warning"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
    }

    #[test]
    fn local_mapping_recommendation_formatting_includes_expected_fields() {
        let rendered = format_local_mapping_recommendation_list(
            &LocalMappingRecommendationList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                recommendations: vec![LocalMappingRecommendation {
                    recommendation_id: Some("rec-1".to_string()),
                    task_id: Some("task-1".to_string()),
                    module_id: Some("chirpstack-primary".to_string()),
                    runtime: Some("container".to_string()),
                    artifact: Some("chirpstack/chirpstack:latest".to_string()),
                    suggested_mapping_key: Some("chirpstack-primary".to_string()),
                    suggested_artifact: Some("chirpstack/chirpstack:latest".to_string()),
                    recommendation_status: Some("proposed".to_string()),
                    priority: Some("high".to_string()),
                    reason: Some(
                        "trusted local mapping is required before assisted execution can proceed"
                            .to_string(),
                    ),
                    safe_to_auto_apply: Some(false),
                    requires_operator: Some(true),
                    ..LocalMappingRecommendation::default()
                }],
                ..LocalMappingRecommendationList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("Local mapping recommendations for node fog-01"));
        assert!(rendered.contains("Recommendations: 1"));
        assert!(rendered.contains("[PROPOSED][HIGH] chirpstack-primary"));
        assert!(rendered.contains("recommendation_id: rec-1"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("module_id: chirpstack-primary"));
        assert!(rendered.contains("artifact: chirpstack/chirpstack:latest"));
        assert!(rendered.contains("suggested mapping key: chirpstack-primary"));
        assert!(rendered.contains("safe to auto apply: false"));
        assert!(rendered.contains("requires operator: true"));
        assert!(rendered.contains(
            "reason: trusted local mapping is required before assisted execution can proceed"
        ));
    }

    #[test]
    fn empty_local_mapping_recommendation_list_formats_clearly() {
        let rendered = format_local_mapping_recommendation_list(
            &LocalMappingRecommendationList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(0),
                recommendations: vec![],
                ..LocalMappingRecommendationList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("No local mapping recommendations for this node."));
        assert!(rendered.contains(
            "Local mapping recommendations are advisory only. This command did not write config and did not execute installation."
        ));
    }

    #[test]
    fn local_mapping_recommendation_json_preserves_raw_response() {
        let response = LocalMappingRecommendationList {
            project_id: Some("smart-city-campinas".to_string()),
            node_id: Some("fog-01".to_string()),
            count: Some(1),
            recommendations: vec![LocalMappingRecommendation {
                recommendation_id: Some("rec-1".to_string()),
                safe_to_auto_apply: Some(false),
                requires_operator: Some(true),
                ..LocalMappingRecommendation::default()
            }],
            ..LocalMappingRecommendationList::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"recommendation_id\": \"rec-1\""));
        assert!(rendered.contains("\"safe_to_auto_apply\": false"));
        assert!(rendered.contains("\"requires_operator\": true"));
    }

    #[test]
    fn local_mapping_patch_proposal_formatting_includes_expected_fields() {
        let rendered = format_local_mapping_patch_proposal_list(
            &LocalMappingPatchProposalList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(1),
                proposals: vec![LocalMappingPatchProposal {
                    proposal_id: Some("proposal-1".to_string()),
                    proposal_status: Some("proposed".to_string()),
                    proposal_kind: Some("add_container_mapping".to_string()),
                    task_id: Some("task-1".to_string()),
                    module_id: Some("chirpstack-primary".to_string()),
                    runtime: Some("container".to_string()),
                    artifact: Some("chirpstack/chirpstack:latest".to_string()),
                    suggested_mapping_key: Some("chirpstack-primary".to_string()),
                    proposed_config_section: Some("dispatch.container_mappings".to_string()),
                    proposed_config_snippet: Some("[dispatch.container_mappings]\n\"chirpstack-primary\" = \"REVIEW_AND_SET_CONTAINER_NAME\"".to_string()),
                    safe_to_auto_apply: Some(false),
                    side_effects_performed: Some(false),
                    requires_operator: Some(true),
                    reason: Some("trusted local mapping is required before assisted execution can proceed".to_string()),
                    ..LocalMappingPatchProposal::default()
                }],
                ..LocalMappingPatchProposalList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("Local mapping patch proposals for node fog-01"));
        assert!(rendered.contains("Proposals: 1"));
        assert!(rendered.contains("[PROPOSED] add_container_mapping"));
        assert!(rendered.contains("proposal_id: proposal-1"));
        assert!(rendered.contains("task_id: task-1"));
        assert!(rendered.contains("module: chirpstack-primary"));
        assert!(rendered.contains("artifact: chirpstack/chirpstack:latest"));
        assert!(rendered.contains("suggested mapping key: chirpstack-primary"));
        assert!(rendered.contains("config section: dispatch.container_mappings"));
        assert!(rendered.contains("safe to auto apply: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("requires operator: true"));
        assert!(rendered.contains("Suggested config snippet:"));
        assert!(rendered.contains("REVIEW_AND_SET_CONTAINER_NAME"));
    }

    #[test]
    fn empty_local_mapping_patch_proposal_list_formats_clearly() {
        let rendered = format_local_mapping_patch_proposal_list(
            &LocalMappingPatchProposalList {
                project_id: Some("smart-city-campinas".to_string()),
                node_id: Some("fog-01".to_string()),
                count: Some(0),
                proposals: vec![],
                ..LocalMappingPatchProposalList::default()
            },
            "smart-city-campinas",
            "fog-01",
        );

        assert!(rendered.contains("No local mapping patch proposals for this node."));
        assert!(rendered.contains(
            "Note: this proposal viewer is advisory only. This command did not write config, did not apply mappings, and did not execute installation."
        ));
    }

    #[test]
    fn local_mapping_patch_proposal_json_preserves_raw_response() {
        let response = LocalMappingPatchProposalList {
            project_id: Some("smart-city-campinas".to_string()),
            node_id: Some("fog-01".to_string()),
            count: Some(1),
            proposals: vec![LocalMappingPatchProposal {
                proposal_id: Some("proposal-1".to_string()),
                proposed_config_snippet: Some(
                    "[dispatch.container_mappings]\n\"chirpstack-primary\" = \"REVIEW_AND_SET_CONTAINER_NAME\""
                        .to_string(),
                ),
                recommendation_snapshot: Some(serde_json::json!({"recommendation_id":"rec-1"})),
                preflight_item_snapshot: Some(serde_json::json!({"task_id":"task-1"})),
                ..LocalMappingPatchProposal::default()
            }],
            ..LocalMappingPatchProposalList::default()
        };

        let rendered = serde_json::to_string_pretty(&response).expect("json should render");

        assert!(rendered.contains("\"proposal_id\": \"proposal-1\""));
        assert!(rendered.contains("\"proposed_config_snippet\""));
        assert!(rendered.contains("\"recommendation_snapshot\""));
        assert!(rendered.contains("\"preflight_item_snapshot\""));
    }

    #[test]
    fn local_mapping_patch_revalidation_formatting_includes_expected_fields() {
        let rendered = format_local_mapping_patch_revalidation_report(
            &LocalMappingPatchRevalidationReport {
                proposal_path: "generated/local-mapping-proposal.toml".to_string(),
                config_path: "config.toml".to_string(),
                status: "warning".to_string(),
                items: vec![LocalMappingPatchRevalidationItem {
                    mapping_key: Some("chirpstack-primary".to_string()),
                    proposed_container_name: Some("REVIEW_AND_SET_CONTAINER_NAME".to_string()),
                    current_container_name: None,
                    status: "incomplete".to_string(),
                    reason: "operator must replace REVIEW_AND_SET_CONTAINER_NAME before any manual config update".to_string(),
                    source_line: Some("\"chirpstack-primary\" = \"REVIEW_AND_SET_CONTAINER_NAME\"".to_string()),
                }],
                summary: LocalMappingPatchRevalidationSummary {
                    total_items: 1,
                    incomplete: 1,
                    ..LocalMappingPatchRevalidationSummary::default()
                },
                safe_to_auto_apply: false,
                side_effects_performed: false,
                applied: false,
                ..LocalMappingPatchRevalidationReport::default()
            },
        );

        assert!(rendered.contains("Local mapping patch revalidation"));
        assert!(rendered.contains("Config: config.toml"));
        assert!(rendered.contains("Proposal: generated/local-mapping-proposal.toml"));
        assert!(rendered.contains("Status: warning"));
        assert!(rendered.contains("Items: 1"));
        assert!(rendered.contains("Safe to auto apply: false"));
        assert!(rendered.contains("Side effects performed: false"));
        assert!(rendered.contains("Applied: false"));
        assert!(rendered.contains("[INCOMPLETE] chirpstack-primary"));
        assert!(rendered.contains("proposed container: REVIEW_AND_SET_CONTAINER_NAME"));
        assert!(rendered.contains(
            "Note: this command did not modify config.toml, did not apply mappings, and did not execute installation."
        ));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            approvals: true,
            approval: true,
            approved_by: Some("operator"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_accepts_approval_mode() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            approve: true,
            approved_by: Some("operator"),
            expires_at_utc: Some("2026-06-01T00:00:00Z"),
            ..ModeCase::default()
        });

        assert!(result.is_ok());
    }

    #[test]
    fn cli_mode_validation_rejects_readiness_with_approval_modes() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            readiness: true,
            approval: true,
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_execution_request_ambiguity() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            execution_request: true,
            request_assisted_execution: true,
            requested_by: Some("operator"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_claim_ambiguity() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            request_id: Some("request-1"),
            execution_claims: true,
            claim_assisted_execution: true,
            claimed_by: Some("operator"),
            expires_at_utc: Some("2026-06-01T00:00:00Z"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_claim_status_ambiguity() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            request_id: Some("request-1"),
            execution_claim: true,
            claim_status: Some("released"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_attempt_ambiguity() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            request_id: Some("request-1"),
            claim_id: Some("claim-1"),
            execution_attempt: true,
            attempt_assisted_execution: true,
            execution_mode: Some("no_op"),
            attempted_by: Some("operator"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_execution_state_ambiguity() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            execution_state: true,
            readiness: true,
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_accepts_execution_state_for_single_task() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            execution_state: true,
            ..ModeCase::default()
        });

        assert!(result.is_ok());
    }

    #[test]
    fn cli_mode_validation_rejects_local_mapping_preflight_ambiguity() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            local_mapping_preflight: true,
            execution_state: true,
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_publish_local_mapping_preflight_without_preflight() {
        let result = validate_command_mode(
            None, None, None, false, true, false, false, None, false, None, false, false, false,
            false, false, false, false, false, None, false, false, false, None, false, false,
            false, None, false, None, None, None, None, None, None, None,
        );

        assert!(result.is_err());
    }

    #[test]
    fn local_mapping_preflight_alone_does_not_require_publish_mode() {
        let result = validate_mode_case(ModeCase {
            local_mapping_preflight: true,
            ..ModeCase::default()
        });

        assert!(result.is_ok());
    }

    #[test]
    fn cli_mode_validation_rejects_local_mapping_recommendations_with_task_id() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            local_mapping_recommendations: true,
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_local_mapping_patch_proposals_with_task_id() {
        let result = validate_mode_case(ModeCase {
            task_id: Some("task-1"),
            local_mapping_patch_proposals: true,
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_write_local_mapping_patch_without_patch_proposals_mode() {
        let result = validate_mode_case(ModeCase {
            write_local_mapping_patch: Some("generated/local-mapping-proposal.toml"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_overwrite_local_mapping_patch_without_write_mode() {
        let result = validate_mode_case(ModeCase {
            overwrite_local_mapping_patch: true,
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_rejects_combining_write_and_revalidate_local_mapping_patch() {
        let result = validate_mode_case(ModeCase {
            write_local_mapping_patch: Some("generated/local-mapping-proposal.toml"),
            revalidate_local_mapping_patch: Some("generated/local-mapping-proposal.toml"),
            ..ModeCase::default()
        });

        assert!(result.is_err());
    }

    #[test]
    fn cli_mode_validation_accepts_revalidate_local_mapping_patch_without_server_mode() {
        let result = validate_mode_case(ModeCase {
            revalidate_local_mapping_patch: Some("generated/local-mapping-proposal.toml"),
            ..ModeCase::default()
        });

        assert!(result.is_ok());
    }

    #[test]
    fn local_mapping_recommendation_viewer_source_guard_has_no_docker_shell_or_config_writes() {
        let source = include_str!("installation_tasks.rs");
        let start = source
            .find("if local_mapping_recommendations {")
            .expect("local mapping recommendation branch should exist");
        let end = source[start..]
            .find("\n    if local_mapping_patch_proposals {")
            .map(|offset| start + offset)
            .expect("patch proposal branch should follow recommendation branch");
        let implementation = &source[start..end];

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "installation task CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }

    #[test]
    fn local_mapping_patch_proposal_viewer_source_guard_has_no_docker_shell_or_config_writes() {
        let source = include_str!("installation_tasks.rs");
        let start = source
            .find("if local_mapping_patch_proposals {")
            .expect("local mapping patch proposal branch should exist");
        let end = source[start..]
            .find("\n    if execution_state {")
            .map(|offset| start + offset)
            .expect("execution_state branch should follow patch proposal branch");
        let implementation = &source[start..end];

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "installation task CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }

    #[test]
    fn local_mapping_patch_revalidation_source_guard_has_no_docker_shell_or_config_writes() {
        let source = include_str!("installation_tasks.rs");
        let start = source
            .find("if let Some(proposal_path) = revalidate_local_mapping_patch {")
            .expect("local mapping patch revalidation branch should exist");
        let end = source[start..]
            .find("\n    if local_mapping_preflight {")
            .map(|offset| start + offset)
            .expect("local mapping preflight branch should follow revalidation branch");
        let implementation = &source[start..end];

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "installation task CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }

    #[test]
    fn local_mapping_preflight_publisher_source_guard_has_no_docker_shell_or_config_writes() {
        let source = include_str!("installation_tasks.rs");
        let start = source
            .find("if local_mapping_preflight {")
            .expect("local mapping preflight branch should exist");
        let end = source[start..]
            .find("\n    if local_mapping_recommendations {")
            .map(|offset| start + offset)
            .expect("recommendation branch should follow preflight branch");
        let implementation = &source[start..end];

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "installation task CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }

    #[test]
    fn execution_state_viewer_source_guard_has_no_docker_shell_or_config_writes() {
        let source = include_str!("installation_tasks.rs");
        let start = source
            .find("if execution_state {")
            .expect("execution_state branch should exist");
        let end = source[start..]
            .find("\n    if execution_claims {")
            .map(|offset| start + offset)
            .expect("execution_claims branch should follow execution_state branch");
        let implementation = &source[start..end];

        for forbidden in [
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "Command::new",
            "std::process::Command",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "installation task CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
