use crate::config::Config;
use crate::dispatch::{
    container_executor::{ensure_container_running_guarded, ContainerExecutionOutcome},
    model::DispatchResponse,
    validator::{validate_dispatch_action, DispatchActionValidation},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchExecutionReport {
    pub local_node_id: String,
    pub execute_requested: bool,
    pub side_effects_enabled: bool,
    pub total_actions: usize,
    pub valid_actions: usize,
    pub skipped_actions: usize,
    pub rejected_actions: usize,
    pub side_effects_performed: bool,
    pub action_results: Vec<DispatchActionExecutionResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchActionExecutionResult {
    pub action_id: Option<String>,
    pub action_kind: Option<String>,
    pub validation_status: String,
    pub execution_status: String,
    pub message: String,
    pub side_effects_performed: Option<bool>,
}

pub fn execute_dispatch_dry_run(
    response: &DispatchResponse,
    local_node_id: &str,
    execute_requested: bool,
    side_effects_enabled: bool,
    config: &Config,
) -> DispatchExecutionReport {
    execute_dispatch_with_runner(
        response,
        local_node_id,
        execute_requested,
        side_effects_enabled,
        config,
        &|action, local_node_id, config| {
            ensure_container_running_guarded(action, local_node_id, config)
        },
    )
}

fn execute_dispatch_with_runner<F>(
    response: &DispatchResponse,
    local_node_id: &str,
    execute_requested: bool,
    side_effects_enabled: bool,
    config: &Config,
    ensure_runner: &F,
) -> DispatchExecutionReport
where
    F: Fn(&crate::dispatch::model::DispatchAction, &str, &Config) -> ContainerExecutionOutcome,
{
    let mut valid_actions = 0;
    let mut skipped_actions = 0;
    let mut rejected_actions = 0;
    let mut side_effects_performed = false;

    let action_results = response
        .actions
        .iter()
        .map(|action| {
            let validation = validate_dispatch_action(action, local_node_id);
            let action_id = action.action_id.clone();
            let action_kind = action.action_kind.clone();

            match validation {
                DispatchActionValidation::Valid => {
                    valid_actions += 1;
                    let (execution_status, message, performed_side_effect) = if !execute_requested {
                        (
                            "dry_run".to_string(),
                            build_dry_run_message(action.artifact.as_deref()),
                            false,
                        )
                    } else if !side_effects_enabled {
                        (
                            "blocked".to_string(),
                            build_blocked_message(action.artifact.as_deref()),
                            false,
                        )
                    } else {
                        match ensure_runner(action, local_node_id, config) {
                            ContainerExecutionOutcome::AlreadyRunning { container_name } => (
                                "already_running".to_string(),
                                format!("container already running: {container_name}"),
                                false,
                            ),
                            ContainerExecutionOutcome::Started { container_name } => (
                                "started".to_string(),
                                format!("started existing container: {container_name}"),
                                true,
                            ),
                            ContainerExecutionOutcome::NotInstalled { reason } => {
                                ("not_installed".to_string(), reason, false)
                            }
                            ContainerExecutionOutcome::MissingLocalMapping { reason } => {
                                ("missing_local_mapping".to_string(), reason, false)
                            }
                            ContainerExecutionOutcome::DockerUnavailable { reason } => {
                                ("docker_unavailable".to_string(), reason, false)
                            }
                            ContainerExecutionOutcome::Failed { reason } => {
                                ("failed".to_string(), reason, false)
                            }
                        }
                    };

                    if performed_side_effect {
                        side_effects_performed = true;
                    }

                    DispatchActionExecutionResult {
                        action_id,
                        action_kind,
                        validation_status: "valid".to_string(),
                        execution_status,
                        message,
                        side_effects_performed: Some(performed_side_effect),
                    }
                }
                DispatchActionValidation::Skipped { reason } => {
                    skipped_actions += 1;
                    DispatchActionExecutionResult {
                        action_id,
                        action_kind,
                        validation_status: "skipped".to_string(),
                        execution_status: "skipped".to_string(),
                        message: reason,
                        side_effects_performed: None,
                    }
                }
                DispatchActionValidation::Rejected { reason } => {
                    rejected_actions += 1;
                    DispatchActionExecutionResult {
                        action_id,
                        action_kind,
                        validation_status: "rejected".to_string(),
                        execution_status: "rejected".to_string(),
                        message: reason,
                        side_effects_performed: None,
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    DispatchExecutionReport {
        local_node_id: local_node_id.to_string(),
        execute_requested,
        side_effects_enabled,
        total_actions: response.actions.len(),
        valid_actions,
        skipped_actions,
        rejected_actions,
        side_effects_performed,
        action_results,
    }
}

fn build_dry_run_message(artifact: Option<&str>) -> String {
    let artifact = artifact.unwrap_or("<missing-artifact>");
    format!("would ensure container running: {artifact}")
}

fn build_blocked_message(artifact: Option<&str>) -> String {
    let artifact = artifact.unwrap_or("<missing-artifact>");
    format!(
        "execution was requested, but side effects are disabled by configuration: would ensure container running: {artifact}"
    )
}

#[cfg(test)]
mod tests {
    use super::{execute_dispatch_with_runner, DispatchExecutionReport};
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, OutputsConfig},
        dispatch::{
            container_executor::ContainerExecutionOutcome,
            model::{DispatchAction, DispatchResponse, DispatchSummary},
        },
    };
    use std::{cell::RefCell, rc::Rc};

    fn valid_response() -> DispatchResponse {
        DispatchResponse {
            project_id: Some("smart-city-campinas".to_string()),
            node_id: Some("fog-01".to_string()),
            summary: Some(DispatchSummary {
                total_actions: Some(1),
                assigned_actions: Some(1),
                ..DispatchSummary::default()
            }),
            actions: vec![DispatchAction {
                action_id: Some("gap:1".to_string()),
                action_kind: Some("ensure_container_running".to_string()),
                runtime: Some("container".to_string()),
                artifact: Some("chirpstack/chirpstack:latest".to_string()),
                target_node: Some("fog-01".to_string()),
                assigned_node: Some("fog-01".to_string()),
                ..DispatchAction::default()
            }],
            ..DispatchResponse::default()
        }
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
                http: None,
            },
            dispatch: Some(DispatchConfig::default()),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    fn run_with_outcome(
        response: &DispatchResponse,
        execute_requested: bool,
        side_effects_enabled: bool,
        outcome: ContainerExecutionOutcome,
        call_counter: Rc<RefCell<usize>>,
    ) -> DispatchExecutionReport {
        execute_dispatch_with_runner(
            response,
            "fog-01",
            execute_requested,
            side_effects_enabled,
            &base_config(),
            &move |_, _, _| {
                *call_counter.borrow_mut() += 1;
                outcome.clone()
            },
        )
    }

    #[test]
    fn execute_false_produces_dry_run_for_valid_action() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            false,
            false,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.valid_actions, 1);
        assert_eq!(report.action_results[0].execution_status, "dry_run");
        assert_eq!(*calls.borrow(), 0);
        assert!(report.action_results[0]
            .message
            .contains("would ensure container running: chirpstack/chirpstack:latest"));
    }

    #[test]
    fn execute_true_with_side_effects_disabled_produces_blocked() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            false,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.action_results[0].execution_status, "blocked");
        assert_eq!(*calls.borrow(), 0);
        assert!(report.action_results[0]
            .message
            .contains("side effects are disabled by configuration"));
    }

    #[test]
    fn execute_true_with_side_effects_enabled_calls_runner() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            true,
            ContainerExecutionOutcome::AlreadyRunning {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.action_results[0].execution_status, "already_running");
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn skipped_validation_produces_skipped_status() {
        let mut response = valid_response();
        response.actions[0].runtime = Some("wasm".to_string());
        response.actions[0].action_kind = Some("ensure_wasm_running".to_string());
        let calls = Rc::new(RefCell::new(0));

        let report = run_with_outcome(
            &response,
            false,
            false,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.skipped_actions, 1);
        assert_eq!(report.action_results[0].execution_status, "skipped");
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn rejected_validation_produces_rejected_status() {
        let mut response = valid_response();
        response.actions[0].assigned_node = Some("fog-02".to_string());
        let calls = Rc::new(RefCell::new(0));

        let report = run_with_outcome(
            &response,
            false,
            false,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.rejected_actions, 1);
        assert_eq!(report.action_results[0].execution_status, "rejected");
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn report_counters_are_correct() {
        let mut response = valid_response();
        response.actions.push(DispatchAction {
            action_id: Some("gap:2".to_string()),
            action_kind: Some("ensure_wasm_running".to_string()),
            runtime: Some("wasm".to_string()),
            target_node: Some("fog-01".to_string()),
            ..DispatchAction::default()
        });
        response.actions.push(DispatchAction {
            action_id: Some("gap:3".to_string()),
            action_kind: Some("ensure_container_running".to_string()),
            runtime: Some("container".to_string()),
            artifact: Some("x".to_string()),
            assigned_node: Some("fog-99".to_string()),
            target_node: Some("fog-01".to_string()),
            ..DispatchAction::default()
        });

        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &response,
            false,
            false,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.total_actions, 3);
        assert_eq!(report.valid_actions, 1);
        assert_eq!(report.skipped_actions, 1);
        assert_eq!(report.rejected_actions, 1);
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn empty_actions_produces_empty_report() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &DispatchResponse::default(),
            false,
            false,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls.clone(),
        );

        assert_eq!(report.total_actions, 0);
        assert_eq!(report.valid_actions, 0);
        assert_eq!(report.skipped_actions, 0);
        assert_eq!(report.rejected_actions, 0);
        assert!(report.action_results.is_empty());
        assert_eq!(*calls.borrow(), 0);
    }

    #[test]
    fn started_outcome_sets_side_effects_performed_true() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            true,
            ContainerExecutionOutcome::Started {
                container_name: "chirpstack".to_string(),
            },
            calls,
        );

        assert_eq!(report.action_results[0].execution_status, "started");
        assert!(report.side_effects_performed);
        assert_eq!(report.action_results[0].side_effects_performed, Some(true));
    }

    #[test]
    fn missing_local_mapping_maps_correctly() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            true,
            ContainerExecutionOutcome::MissingLocalMapping {
                reason: "no trusted local container mapping found for module chirpstack-primary"
                    .to_string(),
            },
            calls,
        );

        assert_eq!(
            report.action_results[0].execution_status,
            "missing_local_mapping"
        );
        assert!(!report.side_effects_performed);
    }

    #[test]
    fn not_installed_maps_correctly() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            true,
            ContainerExecutionOutcome::NotInstalled {
                reason:
                    "trusted container mapping exists, but container is not installed locally: chirpstack"
                        .to_string(),
            },
            calls,
        );

        assert_eq!(report.action_results[0].execution_status, "not_installed");
    }

    #[test]
    fn docker_unavailable_maps_correctly() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            true,
            ContainerExecutionOutcome::DockerUnavailable {
                reason: "Docker is not available or daemon is unreachable".to_string(),
            },
            calls,
        );

        assert_eq!(
            report.action_results[0].execution_status,
            "docker_unavailable"
        );
    }

    #[test]
    fn failed_maps_correctly() {
        let calls = Rc::new(RefCell::new(0));
        let report = run_with_outcome(
            &valid_response(),
            true,
            true,
            ContainerExecutionOutcome::Failed {
                reason: "failed to start existing container 'chirpstack': boom".to_string(),
            },
            calls,
        );

        assert_eq!(report.action_results[0].execution_status, "failed");
    }
}
