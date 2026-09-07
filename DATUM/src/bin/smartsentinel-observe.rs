use std::fs;

use anyhow::{Context, Result};
use clap::Parser;
use datum::config::load_config;
use datum::core::cycle::run_cycle_for_node_identity;
use datum::outputs::http_json::HttpJsonOutput;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-observe")]
#[command(about = "Collect and publish one read-only SmartSentinel observation cycle")]
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

    #[arg(long, default_value_t = 10)]
    timeout_s: u64,

    #[arg(long, default_value = "smartsentinel-observe.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = load_config(&args.config)?;

    let node_id = args
        .node_id
        .or_else(|| Some(cfg.agent.node_id.clone()))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context("SmartSentinel observation requires --node-id or agent.node_id")?;

    let project_id = args
        .project_id
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .and_then(|http| http.project_id.clone())
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context("SmartSentinel observation requires --project-id or outputs.http.project_id")?;

    let snapshot = run_cycle_for_node_identity(
        &node_id,
        args.hostname.as_deref(),
        &cfg.agent.schema_version,
        &cfg,
    )?;

    fs::write(
        &args.out,
        format!("{}\n", serde_json::to_string_pretty(&snapshot)?),
    )
    .with_context(|| format!("write observation snapshot {}", args.out))?;

    let url = format!(
        "{}/api/v1/snapshots",
        args.dserver_url.trim().trim_end_matches('/')
    );
    let output = HttpJsonOutput::new(url, project_id, args.timeout_s)?;
    output.emit(&snapshot)?;

    println!(
        "mode=smartsentinel-observe status=ok node_id={} cycle_id={} entities={} edges={} events={} actions=0 side_effects_performed=false snapshot_published=true out={}",
        snapshot.meta.node_id,
        snapshot.meta.cycle_id,
        snapshot.entities.len(),
        snapshot.edges.len(),
        snapshot.events.len(),
        args.out
    );

    Ok(())
}
