use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use datum::agent::service_loop::{run_service_loop, ServiceLoopOptions};

#[derive(Parser, Debug)]
#[command(name = "smartsentinel")]
#[command(about = "SmartSentinel continuous observability agent")]
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
}

fn main() -> Result<()> {
    let args = Args::parse();

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
