use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};

use crate::{
    action_results::{
        builder::build_action_execution_report_from_dispatch,
        client::publish_action_execution_report,
    },
    config::{Config, DispatchConfig, HttpConfig},
    dispatch::executor::execute_dispatch_dry_run,
    dispatch::model::DispatchResponse,
    dispatch::validator::{validate_dispatch_action, DispatchActionValidation},
};

pub fn run_dispatch_command(
    cfg: &Config,
    execute_flag: bool,
    publish_results_flag: bool,
) -> Result<()> {
    let runtime_cfg = resolve_dispatch_runtime_config(cfg, publish_results_flag)?;

    let client = Client::builder()
        .timeout(Duration::from_secs(runtime_cfg.timeout_s))
        .build()
        .context("build blocking reqwest client")?;

    let response = client
        .get(runtime_cfg.url.clone())
        .header("X-Datum-Project", &runtime_cfg.project_id)
        .send()
        .with_context(|| format!("fetch dispatch from {}", runtime_cfg.url))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(anyhow!(
            "dispatch request failed: status={} body={}",
            status,
            body
        ));
    }

    let dispatch: DispatchResponse =
        serde_json::from_str(&body).context("parse dispatch response JSON")?;
    let execute_requested = execute_flag || runtime_cfg.execute_default;
    let publish_report = execute_dispatch_dry_run(&dispatch, &cfg.agent.node_id, false, false, cfg);

    print_dispatch(&dispatch, &cfg.agent.node_id, execute_requested);

    if runtime_cfg.publish_results {
        let action_report = build_action_execution_report_from_dispatch(
            &dispatch,
            &publish_report,
            &runtime_cfg.project_id,
            &cfg.agent.node_id,
        );
        let response = publish_action_execution_report(
            &runtime_cfg.publish_base_url,
            &runtime_cfg.project_id,
            &cfg.agent.node_id,
            &action_report,
        )?;

        println!();
        println!("Action result publish: ok");
        println!(
            "  report_id: {}",
            response
                .get("report_id")
                .and_then(|v| v.as_str())
                .unwrap_or("<server-defined>")
        );
    } else if publish_results_flag {
        unreachable!("publish_results_flag implies runtime publish_results");
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct DispatchRuntimeConfig {
    url: Url,
    publish_base_url: String,
    project_id: String,
    timeout_s: u64,
    execute_default: bool,
    publish_results: bool,
}

fn resolve_dispatch_runtime_config(
    cfg: &Config,
    publish_results_flag: bool,
) -> Result<DispatchRuntimeConfig> {
    let dispatch_cfg = cfg.dispatch.clone().unwrap_or_default();
    let http_cfg = cfg
        .outputs
        .http
        .as_ref()
        .filter(|http| http.enabled)
        .ok_or_else(|| {
            anyhow!("dispatch.url or outputs.http.enabled=true is required for dispatch")
        })?;

    let project_id = http_cfg
        .project_id
        .clone()
        .unwrap_or_else(|| "default-project".to_string());
    let publish_base_url = http_cfg.url.clone();
    let publish_results = publish_results_flag || dispatch_cfg.publish_results.unwrap_or(false);

    let url = match dispatch_cfg.url.as_deref() {
        Some(url) if !url.trim().is_empty() => {
            dispatch_url_from_dispatch_config(&dispatch_cfg, &cfg.agent.node_id)?
        }
        _ => dispatch_url_for_node(http_cfg, &cfg.agent.node_id)?,
    };

    Ok(DispatchRuntimeConfig {
        url,
        publish_base_url,
        project_id,
        timeout_s: dispatch_cfg
            .timeout_s
            .unwrap_or_else(|| http_cfg.timeout_s.unwrap_or(10)),
        execute_default: dispatch_cfg.execute.unwrap_or(false),
        publish_results,
    })
}

fn print_dispatch(dispatch: &DispatchResponse, local_node_id: &str, execute_requested: bool) {
    let node_id = dispatch.node_id.as_deref().unwrap_or("<unknown>");
    let project_id = dispatch.project_id.as_deref().unwrap_or("<unknown>");
    let assigned_actions = dispatch
        .summary
        .as_ref()
        .map(|summary| summary.assigned_count(dispatch.actions.len()))
        .unwrap_or(dispatch.actions.len());

    println!("Dispatch for node {}", node_id);
    println!("Project: {}", project_id);
    println!("Assigned actions: {}", assigned_actions);
    println!();

    if execute_requested {
        println!("dispatch execution is not implemented yet");
        println!();
    }

    if dispatch.actions.is_empty() {
        println!("No assigned actions.");
        print_validation_report(&DispatchValidationSummary::default());
        return;
    }

    let mut summary = DispatchValidationSummary::default();

    for action in &dispatch.actions {
        let validation = validate_dispatch_action(action, local_node_id);
        summary.total_actions += 1;

        match &validation {
            DispatchActionValidation::Valid => summary.valid_actions += 1,
            DispatchActionValidation::Skipped { .. } => summary.skipped_actions += 1,
            DispatchActionValidation::Rejected { .. } => summary.rejected_actions += 1,
        }

        println!(
            "Action: {}",
            action.action_id.as_deref().unwrap_or("<missing-action-id>")
        );
        println!(
            "  kind: {}",
            action
                .action_kind
                .as_deref()
                .unwrap_or("<missing-action-kind>")
        );
        println!(
            "  runtime: {}",
            action.runtime.as_deref().unwrap_or("<unknown>")
        );
        println!(
            "  artifact: {}",
            action.artifact.as_deref().unwrap_or("<none>")
        );
        println!("  assignment: {}", assignment_label(action));
        println!("  validation: {}", validation_label(&validation));

        match &validation {
            DispatchActionValidation::Valid => {}
            DispatchActionValidation::Skipped { reason }
            | DispatchActionValidation::Rejected { reason } => {
                println!("  reason: {}", reason);
            }
        }

        println!();
    }

    print_validation_report(&summary);
}

fn validation_label(result: &DispatchActionValidation) -> &'static str {
    match result {
        DispatchActionValidation::Valid => "valid",
        DispatchActionValidation::Skipped { .. } => "skipped",
        DispatchActionValidation::Rejected { .. } => "rejected",
    }
}

fn assignment_label(action: &crate::dispatch::model::DispatchAction) -> &'static str {
    if action.assigned_node.is_some() {
        "assigned_node"
    } else if action.target_node.is_some() {
        "target_node"
    } else {
        "unscoped"
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct DispatchValidationSummary {
    total_actions: usize,
    valid_actions: usize,
    skipped_actions: usize,
    rejected_actions: usize,
}

fn print_validation_report(summary: &DispatchValidationSummary) {
    println!("Validation report:");
    println!("  total actions: {}", summary.total_actions);
    println!("  valid: {}", summary.valid_actions);
    println!("  skipped: {}", summary.skipped_actions);
    println!("  rejected: {}", summary.rejected_actions);
}

fn dispatch_url_from_dispatch_config(dispatch_cfg: &DispatchConfig, node_id: &str) -> Result<Url> {
    let template = dispatch_cfg
        .url
        .as_deref()
        .ok_or_else(|| anyhow!("dispatch.url is required when [dispatch] is configured"))?;

    let resolved = template.replace("{node_id}", node_id);
    Url::parse(&resolved).with_context(|| format!("parse dispatch.url: {resolved}"))
}

fn dispatch_url_for_node(http_cfg: &HttpConfig, node_id: &str) -> Result<Url> {
    let mut url = Url::parse(&http_cfg.url)
        .with_context(|| format!("parse outputs.http.url: {}", http_cfg.url))?;

    let mut segments = url
        .path_segments()
        .ok_or_else(|| anyhow!("outputs.http.url must be an absolute HTTP URL"))?
        .map(str::to_string)
        .collect::<Vec<_>>();

    if segments.last().map(|s| s.as_str()) == Some("snapshots") {
        segments.pop();
    }

    segments.extend([
        "dispatch".to_string(),
        "for-node".to_string(),
        node_id.to_string(),
    ]);

    url.path_segments_mut()
        .map_err(|_| anyhow!("outputs.http.url cannot be a base URL"))?
        .clear()
        .extend(segments.iter().map(|s| s.as_str()));

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::{
        dispatch_url_for_node, dispatch_url_from_dispatch_config, resolve_dispatch_runtime_config,
    };
    use crate::config::{AgentConfig, Config, DispatchConfig, HttpConfig, OutputsConfig};

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
            dispatch: None,
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn derives_dispatch_endpoint_from_snapshot_endpoint() {
        let cfg = HttpConfig {
            enabled: true,
            url: "http://localhost:8080/api/v1/snapshots".to_string(),
            project_id: Some("smart-city-campinas".to_string()),
            timeout_s: Some(10),
        };

        let url = dispatch_url_for_node(&cfg, "fog-01").expect("dispatch url should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/dispatch/for-node/fog-01"
        );
    }

    #[test]
    fn preserves_base_path_when_snapshot_suffix_is_missing() {
        let cfg = HttpConfig {
            enabled: true,
            url: "http://localhost:8080/api/v1".to_string(),
            project_id: None,
            timeout_s: None,
        };

        let url = dispatch_url_for_node(&cfg, "fog-01").expect("dispatch url should build");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/dispatch/for-node/fog-01"
        );
    }

    #[test]
    fn runtime_config_uses_output_http_defaults() {
        let runtime = resolve_dispatch_runtime_config(&base_config(), false)
            .expect("runtime config should resolve");

        assert_eq!(runtime.timeout_s, 10);
        assert_eq!(runtime.project_id, "smart-city-campinas");
        assert_eq!(
            runtime.url.as_str(),
            "http://localhost:8080/api/v1/dispatch/for-node/fog-01"
        );
        assert!(!runtime.execute_default);
        assert!(!runtime.publish_results);
    }

    #[test]
    fn expands_dispatch_template_url() {
        let cfg = DispatchConfig {
            enabled: false,
            url: Some("http://localhost:8080/api/v1/dispatch/for-node/{node_id}".to_string()),
            project_id: None,
            timeout_s: None,
            execute: None,
            publish_results: None,
            allow_container_execution: None,
            configuration_root: None,
            secret_bindings: Vec::new(),
            container_mappings: Vec::new(),
        };

        let url = dispatch_url_from_dispatch_config(&cfg, "fog-01")
            .expect("dispatch template should expand");

        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/dispatch/for-node/fog-01"
        );
    }

    #[test]
    fn runtime_config_uses_dispatch_flags() {
        let mut cfg = base_config();
        cfg.dispatch = Some(DispatchConfig {
            enabled: true,
            url: Some("http://localhost:8080/api/v1/dispatch/for-node/{node_id}".to_string()),
            project_id: Some("smart-city-campinas".to_string()),
            timeout_s: Some(12),
            execute: Some(true),
            publish_results: Some(true),
            allow_container_execution: Some(true),
            configuration_root: None,
            secret_bindings: Vec::new(),
            container_mappings: Vec::new(),
        });

        let runtime =
            resolve_dispatch_runtime_config(&cfg, false).expect("runtime config should resolve");

        assert_eq!(runtime.timeout_s, 12);
        assert!(runtime.execute_default);
        assert!(runtime.publish_results);
    }

    #[test]
    fn runtime_config_enables_publish_results_from_cli_flag() {
        let runtime = resolve_dispatch_runtime_config(&base_config(), true)
            .expect("runtime config should resolve");

        assert!(runtime.publish_results);
    }
}
