use crate::config::Config;

const ALLOWED_EXECUTION_MODES: &[&str] = &["no_op", "dry_run"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistedExecutionPreflightReport {
    pub task_id: String,
    pub request_id: String,
    pub claim_id: String,
    pub execution_mode: String,
    pub local_node_id: String,
    pub preflight_status: String,
    pub checks: Vec<AssistedExecutionPreflightCheck>,
    pub message: String,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistedExecutionPreflightCheck {
    pub name: String,
    pub status: String,
    pub message: String,
}

pub fn run_assisted_execution_preflight(
    config: &Config,
    task_id: &str,
    request_id: &str,
    claim_id: &str,
    execution_mode: &str,
) -> AssistedExecutionPreflightReport {
    let effective_cfg = config.assisted_execution_config();
    let local_node_id = config.agent.node_id.trim().to_string();
    let mut checks = Vec::new();

    checks.push(build_execution_mode_check(
        execution_mode.trim(),
        effective_cfg.allow_no_op_attempts,
        effective_cfg.allow_dry_run_attempts,
    ));

    checks.push(if local_node_id.is_empty() {
        AssistedExecutionPreflightCheck {
            name: "local_node_id_resolved".to_string(),
            status: "failed".to_string(),
            message: "local DATUM node_id is empty".to_string(),
        }
    } else {
        AssistedExecutionPreflightCheck {
            name: "local_node_id_resolved".to_string(),
            status: "passed".to_string(),
            message: local_node_id.clone(),
        }
    });

    checks.push(AssistedExecutionPreflightCheck {
        name: "config_loaded".to_string(),
        status: "passed".to_string(),
        message: "client config is loaded in memory".to_string(),
    });

    checks.push(if effective_cfg.enabled {
        AssistedExecutionPreflightCheck {
            name: "assisted_execution_enabled_config_check".to_string(),
            status: "passed".to_string(),
            message: "assisted execution config is enabled for local preflight/no-op handling"
                .to_string(),
        }
    } else {
        AssistedExecutionPreflightCheck {
            name: "assisted_execution_enabled_config_check".to_string(),
            status: "warning".to_string(),
            message:
                "assisted execution is not enabled; metadata-only preflight/no-op attempt only"
                    .to_string(),
        }
    });

    checks.push(if effective_cfg.allow_side_effects {
        AssistedExecutionPreflightCheck {
            name: "side_effects_config_check".to_string(),
            status: "warning".to_string(),
            message:
                "allow_side_effects=true is ignored in this milestone; side effects remain disabled"
                    .to_string(),
        }
    } else {
        AssistedExecutionPreflightCheck {
            name: "side_effects_config_check".to_string(),
            status: "passed".to_string(),
            message: "side effects are disabled for this milestone".to_string(),
        }
    });

    let failure_count = checks
        .iter()
        .filter(|check| check.status == "failed")
        .count();
    let warning_count = checks
        .iter()
        .filter(|check| check.status == "warning")
        .count();

    let preflight_status = if failure_count > 0 {
        "failed"
    } else if warning_count > 0 {
        "warning"
    } else {
        "passed"
    }
    .to_string();

    let message = format!(
        "preflight_status={} checks={} warnings={} failures={} local_node_id={} execution_mode={} safe_to_auto_execute=false side_effects_performed=false",
        preflight_status,
        checks.len(),
        warning_count,
        failure_count,
        if local_node_id.is_empty() {
            "<empty>"
        } else {
            local_node_id.as_str()
        },
        execution_mode.trim()
    );

    AssistedExecutionPreflightReport {
        task_id: task_id.trim().to_string(),
        request_id: request_id.trim().to_string(),
        claim_id: claim_id.trim().to_string(),
        execution_mode: execution_mode.trim().to_string(),
        local_node_id,
        preflight_status,
        checks,
        message,
        safe_to_auto_execute: false,
        side_effects_performed: false,
    }
}

fn build_execution_mode_check(
    execution_mode: &str,
    allow_no_op_attempts: bool,
    allow_dry_run_attempts: bool,
) -> AssistedExecutionPreflightCheck {
    if !ALLOWED_EXECUTION_MODES.contains(&execution_mode) {
        return AssistedExecutionPreflightCheck {
            name: "execution_mode_allowed".to_string(),
            status: "failed".to_string(),
            message: format!(
                "invalid assisted execution mode '{}'; allowed values: {}",
                execution_mode,
                ALLOWED_EXECUTION_MODES.join(", ")
            ),
        };
    }

    match execution_mode {
        "no_op" if !allow_no_op_attempts => AssistedExecutionPreflightCheck {
            name: "execution_mode_allowed".to_string(),
            status: "failed".to_string(),
            message: "no_op attempts are disabled by local assisted execution config".to_string(),
        },
        "dry_run" if !allow_dry_run_attempts => AssistedExecutionPreflightCheck {
            name: "execution_mode_allowed".to_string(),
            status: "failed".to_string(),
            message: "dry_run attempts are disabled by local assisted execution config".to_string(),
        },
        _ => AssistedExecutionPreflightCheck {
            name: "execution_mode_allowed".to_string(),
            status: "passed".to_string(),
            message: format!("{} is allowed", execution_mode),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{run_assisted_execution_preflight, AssistedExecutionPreflightCheck};
    use crate::config::{
        AgentConfig, AssistedExecutionConfig, Config, DispatchConfig, OutputsConfig,
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
            dispatch: Some(DispatchConfig::default()),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    fn check<'a>(
        checks: &'a [AssistedExecutionPreflightCheck],
        name: &str,
    ) -> &'a AssistedExecutionPreflightCheck {
        checks
            .iter()
            .find(|check| check.name == name)
            .expect("check should exist")
    }

    #[test]
    fn preflight_accepts_no_op() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "no_op",
        );

        assert_eq!(report.execution_mode, "no_op");
        assert_ne!(report.preflight_status, "failed");
        assert_eq!(
            check(&report.checks, "execution_mode_allowed").status,
            "passed"
        );
    }

    #[test]
    fn preflight_accepts_dry_run() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "dry_run",
        );

        assert_eq!(report.execution_mode, "dry_run");
        assert_ne!(report.preflight_status, "failed");
        assert_eq!(
            check(&report.checks, "execution_mode_allowed").status,
            "passed"
        );
    }

    #[test]
    fn preflight_rejects_invalid_execution_mode() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "execute",
        );

        assert_eq!(report.preflight_status, "failed");
        assert_eq!(
            check(&report.checks, "execution_mode_allowed").status,
            "failed"
        );
    }

    #[test]
    fn preflight_always_has_safe_to_auto_execute_false() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "no_op",
        );

        assert!(!report.safe_to_auto_execute);
    }

    #[test]
    fn preflight_always_has_side_effects_performed_false() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "dry_run",
        );

        assert!(!report.side_effects_performed);
    }

    #[test]
    fn preflight_includes_expected_checks() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "no_op",
        );

        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "execution_mode_allowed"));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "local_node_id_resolved"));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "config_loaded"));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "assisted_execution_enabled_config_check"));
    }

    #[test]
    fn dry_run_can_be_disabled_by_config() {
        let mut cfg = base_config();
        cfg.assisted_execution = Some(AssistedExecutionConfig {
            enabled: true,
            allow_no_op_attempts: true,
            allow_dry_run_attempts: false,
            allow_side_effects: false,
        });

        let report =
            run_assisted_execution_preflight(&cfg, "task-1", "request-1", "claim-1", "dry_run");

        assert_eq!(report.preflight_status, "failed");
        assert_eq!(
            check(&report.checks, "execution_mode_allowed").status,
            "failed"
        );
    }

    #[test]
    fn no_op_can_be_disabled_by_config() {
        let mut cfg = base_config();
        cfg.assisted_execution = Some(AssistedExecutionConfig {
            enabled: true,
            allow_no_op_attempts: false,
            allow_dry_run_attempts: true,
            allow_side_effects: false,
        });

        let report =
            run_assisted_execution_preflight(&cfg, "task-1", "request-1", "claim-1", "no_op");

        assert_eq!(report.preflight_status, "failed");
        assert_eq!(
            check(&report.checks, "execution_mode_allowed").status,
            "failed"
        );
    }

    #[test]
    fn disabled_executor_produces_warning_not_failure() {
        let report = run_assisted_execution_preflight(
            &base_config(),
            "task-1",
            "request-1",
            "claim-1",
            "no_op",
        );

        assert_eq!(report.preflight_status, "warning");
        assert_eq!(
            check(&report.checks, "assisted_execution_enabled_config_check").status,
            "warning"
        );
    }

    #[test]
    fn implementation_never_uses_side_effect_execution_or_config_writes() {
        let source = include_str!("runner.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);
        let docker_run = ["docker", "run"].join(" ");
        let docker_pull = ["docker", "pull"].join(" ");
        let docker_rm = ["docker", "rm"].join(" ");

        assert!(!implementation.contains(&docker_run));
        assert!(!implementation.contains(&docker_pull));
        assert!(!implementation.contains(&docker_rm));
        assert!(!implementation.contains("Command::new("));
        assert!(!implementation.contains("fs::write"));
        assert!(!implementation.contains("write_to_string"));
    }
}
