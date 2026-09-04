use std::fs;

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::agent::bootstrap::run_bootstrap;
use datum::agent::resource_discovery::load_config_allow_unbound;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-bootstrap")]
#[command(about = "Resolve SmartSentinel bound-node, unbound-resource, or unknown-resource bootstrap against dserver")]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,

    #[arg(long)]
    dserver_url: String,

    #[arg(long)]
    project_id: Option<String>,

    #[arg(long, default_value_t = 5)]
    timeout_s: u64,

    #[arg(long, default_value = "smartsentinel-bootstrap.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let configured_node_id = read_configured_node_id(&args.config)?;
    let cfg = load_config_allow_unbound(&args.config)?;
    let requested_node_id = args
        .node_id
        .or(configured_node_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let project_id = args
        .project_id
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .and_then(|http| http.project_id.clone())
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "SmartSentinel bootstrap requires --project-id or outputs.http.project_id"
        ))?;

    if args.dserver_url.trim().is_empty() {
        bail!("SmartSentinel bootstrap requires a non-empty --dserver-url");
    }

    let report = run_bootstrap(
        &cfg,
        requested_node_id.as_deref(),
        args.hostname.as_deref(),
        &args.dserver_url,
        &project_id,
        args.timeout_s,
    )?;

    let json = serde_json::to_string_pretty(&report)
        .context("serialize SmartSentinel bootstrap report")?;
    fs::write(&args.out, format!("{}\n", json))
        .with_context(|| format!("write SmartSentinel bootstrap report {}", args.out))?;

    println!(
        "mode=smartsentinel-bootstrap status=ok bootstrap_state={:?} resource_id={} requested_node_id={} resolved_node_id={} instance_id={} hardware_id={} capacity_profile_id={} resource_known={} node_known={} out={} side_effects=false",
        report.bootstrap_state,
        report.control_plane.resource_id,
        report
            .control_plane
            .requested_node_id
            .as_deref()
            .unwrap_or("none"),
        report
            .control_plane
            .resolved_node_id
            .as_deref()
            .unwrap_or("none"),
        report.resource_discovery.instance_id,
        report
            .resource_discovery
            .identity
            .hardware_fingerprint_id
            .as_deref()
            .unwrap_or("unknown"),
        report.resource_discovery.resources.capacity_profile_id,
        report.control_plane.resource_record.is_some(),
        report.control_plane.node_known,
        args.out,
    );

    Ok(())
}

fn read_configured_node_id(path: &str) -> Result<Option<String>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {path}"))?;
    let value: toml::Value = toml::from_str(&raw).context("parse bootstrap TOML config")?;
    Ok(value
        .get("agent")
        .and_then(toml::Value::as_table)
        .and_then(|agent| agent.get("node_id"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string))
}
