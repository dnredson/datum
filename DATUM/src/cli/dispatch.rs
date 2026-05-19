use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};

use crate::{
    config::{Config, DispatchConfig, HttpConfig},
    dispatch::executor::{execute_dispatch_dry_run, DispatchExecutionReport},
    dispatch::model::DispatchResponse,
    dispatch::validator::{validate_dispatch_action, DispatchActionValidation},
};

pub fn run_dispatch_command(cfg: &Config, execute_flag: bool) -> Result<()> {
    let runtime_cfg = resolve_dispatch_runtime_config(cfg)?;

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
    let report = execute_dispatch_dry_run(
        &dispatch,
        &cfg.agent.node_id,
        execute_requested,
        runtime_cfg.side_effects_enabled,
        cfg,
    );

    print_dispatch(&dispatch, &cfg.agent.node_id, &report);

    Ok(())
}

#[derive(Debug, Clone)]
struct DispatchRuntimeConfig {
    url: Url,
    project_id: String,
    timeout_s: u64,
    execute_default: bool,
    side_effects_enabled: bool,
}

fn resolve_dispatch_runtime_config(cfg: &Config) -> Result<DispatchRuntimeConfig> {
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

    let url = match dispatch_cfg.url.as_deref() {
        Some(url) if !url.trim().is_empty() => {
            dispatch_url_from_dispatch_config(&dispatch_cfg, &cfg.agent.node_id)?
        }
        _ => dispatch_url_for_node(http_cfg, &cfg.agent.node_id)?,
    };

    Ok(DispatchRuntimeConfig {
        url,
        project_id,
        timeout_s: dispatch_cfg
            .timeout_s
            .unwrap_or_else(|| http_cfg.timeout_s.unwrap_or(10)),
        execute_default: dispatch_cfg.execute.unwrap_or(false),
        side_effects_enabled: dispatch_cfg.allow_container_execution.unwrap_or(false),
    })
}

fn print_dispatch(
    dispatch: &DispatchResponse,
    local_node_id: &str,
    report: &DispatchExecutionReport,
) {
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

    if report.execute_requested && !report.side_effects_enabled {
        println!(
            "dispatch execution was requested, but side effects are disabled by configuration"
        );
        println!();
    } else if report.execute_requested && report.side_effects_enabled {
        println!("dispatch execution pipeline is enabled, but real container/WASM execution is not implemented in C7/30");
        println!();
    }

    if dispatch.actions.is_empty() {
        println!("No assigned actions.");
        print_execution_report(report);
        return;
    }

    for (action, result) in dispatch.actions.iter().zip(report.action_results.iter()) {
        let validation = validate_dispatch_action(action, local_node_id);
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

        match validation {
            DispatchActionValidation::Valid => {}
            DispatchActionValidation::Skipped { reason }
            | DispatchActionValidation::Rejected { reason } => {
                println!("  reason: {}", reason);
            }
        }

        println!("  execution: {}", result.execution_status);
        println!("  result: {}", result.message);

        println!();
    }

    print_execution_report(report);
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

fn print_execution_report(report: &DispatchExecutionReport) {
    println!("Execution report:");
    println!("  total actions: {}", report.total_actions);
    println!("  valid: {}", report.valid_actions);
    println!("  skipped: {}", report.skipped_actions);
    println!("  rejected: {}", report.rejected_actions);
    println!(
        "  side effects performed: {}",
        report.side_effects_performed
    );
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
        let runtime =
            resolve_dispatch_runtime_config(&base_config()).expect("runtime config should resolve");

        assert_eq!(runtime.timeout_s, 10);
        assert_eq!(runtime.project_id, "smart-city-campinas");
        assert_eq!(
            runtime.url.as_str(),
            "http://localhost:8080/api/v1/dispatch/for-node/fog-01"
        );
        assert!(!runtime.execute_default);
        assert!(!runtime.side_effects_enabled);
    }

    #[test]
    fn expands_dispatch_template_url() {
        let cfg = DispatchConfig {
            enabled: false,
            url: Some("http://localhost:8080/api/v1/dispatch/for-node/{node_id}".to_string()),
            project_id: None,
            timeout_s: None,
            execute: None,
            allow_container_execution: None,
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
            allow_container_execution: Some(true),
            container_mappings: Vec::new(),
        });

        let runtime = resolve_dispatch_runtime_config(&cfg).expect("runtime config should resolve");

        assert_eq!(runtime.timeout_s, 12);
        assert!(runtime.execute_default);
        assert!(runtime.side_effects_enabled);
    }
}
