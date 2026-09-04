use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;
use datum::agent::operational_service_loop::{
    run_operational_service_loop, OperationalServiceLoopOptions,
};
use datum::agent::service_loop::{run_service_loop, ServiceLoopOptions};

#[derive(Parser, Debug)]
#[command(name = "smartsentinel")]
#[command(about = "SmartSentinel continuous observability and operational reconciliation agent")]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    cycle_s: Option<u64>,

    #[arg(long, default_value_t = 1)]
    heartbeat_s: u64,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,

    #[arg(long, default_value = ".smartsentinel-stop")]
    stop_file: PathBuf,

    #[arg(long)]
    max_cycles: Option<u64>,

    #[arg(long, default_value_t = false)]
    operational: bool,

    #[arg(long)]
    dserver_url: Option<String>,

    #[arg(long)]
    project_id: Option<String>,

    #[arg(long, default_value_t = 5)]
    timeout_s: u64,

    #[arg(long, default_value_t = false)]
    execute: bool,

    #[arg(long, default_value = "smartsentinel-operational.json")]
    operational_out: PathBuf,

    #[arg(long, default_value = "smartsentinel-operational-history.jsonl")]
    operational_history_out: PathBuf,

    #[arg(long, default_value_t = 30)]
    discovery_refresh_s: u64,

    #[arg(long, default_value_t = 30)]
    snapshot_s: u64,

    #[arg(long, default_value_t = 30)]
    drift_verify_s: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.operational {
        let dserver_url = args
            .dserver_url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--operational requires --dserver-url"))?;
        let project_id = args
            .project_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--operational requires --project-id"))?;

        return run_operational_service_loop(OperationalServiceLoopOptions {
            config_path: args.config,
            dserver_url,
            project_id,
            timeout_s: args.timeout_s,
            execute: args.execute,
            report_path: args.operational_out,
            history_path: args.operational_history_out,
            cycle_s_override: args.cycle_s,
            discovery_refresh_s: args.discovery_refresh_s,
            snapshot_s: args.snapshot_s,
            drift_verify_s: args.drift_verify_s,
            heartbeat_s: args.heartbeat_s,
            node_id_override: args.node_id,
            hostname_override: args.hostname,
            stop_file: args.stop_file,
            max_cycles: args.max_cycles,
        });
    }

    if args.dserver_url.is_some() || args.project_id.is_some() || args.execute {
        bail!("--dserver-url, --project-id, and --execute require --operational");
    }

    run_service_loop(ServiceLoopOptions {
        config_path: args.config,
        out_override: args.out,
        cycle_s_override: args.cycle_s,
        heartbeat_s: args.heartbeat_s,
        node_id_override: args.node_id,
        hostname_override: args.hostname,
        stop_file: args.stop_file,
        max_cycles: args.max_cycles,
    })
}
