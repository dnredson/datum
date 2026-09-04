use std::fs;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use datum::agent::discovery::BootstrapMode;
use datum::agent::discovery_collectors::collect_discovery;
use datum::config;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BootstrapModeArg {
    KnownNode,
    Discovery,
}

impl From<BootstrapModeArg> for BootstrapMode {
    fn from(value: BootstrapModeArg) -> Self {
        match value {
            BootstrapModeArg::KnownNode => BootstrapMode::KnownNode,
            BootstrapModeArg::Discovery => BootstrapMode::Discovery,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-discovery")]
#[command(about = "Collect one observation-only SmartSentinel host/capability discovery report")]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,

    #[arg(long, value_enum, default_value_t = BootstrapModeArg::Discovery)]
    bootstrap_mode: BootstrapModeArg,

    #[arg(long, default_value = "smartsentinel-discovery.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = config::load_config(&args.config)?;
    let node_id = args.node_id.unwrap_or_else(|| cfg.agent.node_id.clone());

    let discovery = collect_discovery(
        &node_id,
        args.hostname.as_deref(),
        args.bootstrap_mode.into(),
        &cfg,
    )?;

    let json = serde_json::to_string_pretty(&discovery).context("serialize SmartSentinel discovery")?;
    fs::write(&args.out, format!("{}\n", json))
        .with_context(|| format!("write SmartSentinel discovery report {}", args.out))?;

    println!(
        "mode=smartsentinel-discovery status=ok node_id={} instance_id={} host_id={} stage={} bootstrap_mode={:?} capabilities={} out={} side_effects=false",
        discovery.node_id,
        discovery.instance_id,
        discovery.host_id,
        discovery.stage.as_deref().unwrap_or("unknown"),
        discovery.bootstrap_mode,
        discovery.capabilities.len(),
        args.out,
    );

    Ok(())
}
