use std::fs;

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::agent::resource_discovery::collect_unbound_resource_discovery;
use datum::config::Config;

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

fn load_config_allow_unbound(path: &str) -> Result<Config> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {path}"))?;
    let mut value: toml::Value = toml::from_str(&raw).context("parse TOML resource config")?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("resource config root must be a TOML table"))?;
    let agent = root
        .get_mut("agent")
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("resource config requires an [agent] table"))?;

    let needs_probe_id = agent
        .get("node_id")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true);
    if needs_probe_id {
        agent.insert(
            "node_id".to_string(),
            toml::Value::String(format!("resource-probe-{}", uuid::Uuid::new_v4())),
        );
    }

    let normalized = toml::to_string(&value).context("normalize role-free resource config")?;
    let cfg: Config = toml::from_str(&normalized).context("decode role-free resource config")?;
    if cfg.agent.schema_version.trim().is_empty() {
        bail!("resource config agent.schema_version must not be empty");
    }
    Ok(cfg)
}
