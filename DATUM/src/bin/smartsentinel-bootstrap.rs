use std::fs;

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::agent::bootstrap::run_bootstrap;
use datum::config;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-bootstrap")]
#[command(about = "Resolve SmartSentinel known-node vs discovery bootstrap against dserver")]
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
    let cfg = config::load_config(&args.config)?;
    let node_id = args.node_id.unwrap_or_else(|| cfg.agent.node_id.clone());
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
        &node_id,
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
        "mode=smartsentinel-bootstrap status=ok node_id={} instance_id={} bootstrap_mode={:?} node_known={} hardware_id={} binding_id={} capacity_profile_id={} expected_capabilities={} existing_registration={} out={} side_effects=false",
        report.discovery.node_id,
        report.discovery.instance_id,
        report.control_plane.bootstrap_mode,
        report.control_plane.node_known,
        report
            .discovery
            .identity
            .hardware_fingerprint_id
            .as_deref()
            .unwrap_or("unknown"),
        report.discovery.identity.role_binding_id,
        report.discovery.resources.capacity_profile_id,
        report.control_plane.expected_capabilities.is_some(),
        report.control_plane.sentinel_registration.is_some(),
        args.out,
    );

    Ok(())
}
