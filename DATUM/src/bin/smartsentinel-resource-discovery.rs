use std::fs;

use anyhow::{Context, Result};
use clap::Parser;
use datum::agent::resource_discovery::{
    collect_unbound_resource_discovery, load_config_allow_unbound,
};

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-resource-discovery")]
#[command(about = "Discover one compute resource without requiring a DATUM role/node_id")]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    hostname: Option<String>,

    #[arg(long, default_value = "smartsentinel-resource-discovery.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = load_config_allow_unbound(&args.config)?;
    let report = collect_unbound_resource_discovery(&cfg, args.hostname.as_deref())?;

    let json = serde_json::to_string_pretty(&report)
        .context("serialize SmartSentinel resource discovery")?;
    fs::write(&args.out, format!("{}\n", json))
        .with_context(|| format!("write SmartSentinel resource discovery {}", args.out))?;

    println!(
        "mode=smartsentinel-resource-discovery status=ok instance_id={} hardware_fingerprint_id={} os_installation_id={} capacity_profile_id={} capabilities={} out={} side_effects=false",
        report.instance_id,
        report
            .identity
            .hardware_fingerprint_id
            .as_deref()
            .unwrap_or("unknown"),
        report
            .identity
            .os_installation_id
            .as_deref()
            .unwrap_or("unknown"),
        report.resources.capacity_profile_id,
        report.capabilities.len(),
        args.out,
    );

    Ok(())
}
