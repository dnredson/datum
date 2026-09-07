use std::{fs, path::PathBuf};

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::config::load_config;
use datum::agent::service_loop::{run_service_loop, ServiceLoopOptions};
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

    /// Keep collecting and publishing until Ctrl+C, a stop file, or max-cycles.
    #[arg(long, default_value_t = false)]
    continuous: bool,

    #[arg(long)]
    cycle_s: Option<u64>,

    #[arg(long, default_value_t = 1)]
    heartbeat_s: u64,

    #[arg(long, default_value = ".smartsentinel-stop")]
    stop_file: PathBuf,

    #[arg(long)]
    max_cycles: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.continuous {
        let project_id = args
            .project_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("--continuous requires --project-id"))?;

        if args.max_cycles == Some(0) {
            bail!("--max-cycles must be greater than zero when provided");
        }
        if args.heartbeat_s == 0 {
            bail!("--heartbeat-s must be greater than zero");
        }

        let publish_url = format!(
            "{}/api/v1/snapshots",
            args.dserver_url.trim().trim_end_matches('/')
        );

        return run_service_loop(ServiceLoopOptions {
            config_path: args.config,
            out_override: Some(args.out),
            cycle_s_override: args.cycle_s,
            heartbeat_s: args.heartbeat_s,
            node_id_override: args.node_id,
            hostname_override: args.hostname,
            stop_file: args.stop_file,
            max_cycles: args.max_cycles,
            http_url_override: Some(publish_url),
            http_project_id_override: Some(project_id),
            http_timeout_s_override: Some(args.timeout_s),
        });
    }

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
