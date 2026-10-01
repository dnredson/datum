use std::fs;

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::agent::operational_reconciliation::run_operational_reconciliation;
use datum::agent::resource_discovery::load_config_allow_unbound;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-operational-reconcile")]
#[command(about = "Resolve a bound role, fetch its operational D-Graph slice, run sandboxed WASM reconciliation, and optionally execute governed host actions")]
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

    #[arg(long, default_value_t = false)]
    execute: bool,

    #[arg(long, default_value_t = false)]
    rollback: bool,

    /// Phase 119G: process ONLY this node's governed source-cleanup
    /// directives (a service historically placed here under some PAST
    /// accepted D-Map, but no longer desired here under CURRENT authority).
    /// Mutually exclusive with --rollback: cleanup never uses
    /// `desired_services` as removal targets, and rollback never touches
    /// `cleanup_directives`.
    #[arg(long, default_value_t = false)]
    cleanup: bool,

    #[arg(long, default_value = "smartsentinel-operational-reconcile.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.dserver_url.trim().is_empty() {
        bail!("SmartSentinel operational reconciliation requires non-empty --dserver-url");
    }
    if args.cleanup && args.rollback {
        bail!("--cleanup and --rollback are mutually exclusive");
    }

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
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "SmartSentinel operational reconciliation requires --project-id or outputs.http.project_id"
        ))?;

    let report = run_operational_reconciliation(
        &cfg,
        requested_node_id.as_deref(),
        args.hostname.as_deref(),
        &args.dserver_url,
        &project_id,
        args.timeout_s,
        args.execute,
        args.rollback,
        args.cleanup,
    )?;

    let json = serde_json::to_string_pretty(&report)
        .context("serialize SmartSentinel operational reconciliation report")?;
    fs::write(&args.out, format!("{}\n", json))
        .with_context(|| format!("write operational reconciliation report {}", args.out))?;

    let wasm_executed = report
        .services
        .iter()
        .filter(|service| service.wasm.status == "executed")
        .count();
    let converged_after = report
        .services
        .iter()
        .filter(|service| service.observed_after_running)
        .count();

    let cleanup_mutated = report.cleanup.iter().filter(|entry| entry.host_mutation_performed).count();

    println!(
        "mode=smartsentinel-operational-reconcile status=ok bootstrap_state={:?} resource_id={} node_id={} dgraph_id={} services={} wasm_executed={} execute_requested={} rollback_requested={} cleanup_requested={} control_plane_authorized={} local_container_execution_allowed={} converged_after={} host_mutation={} cleanup_directives={} cleanup_mutated={} snapshot_published={} out={}",
        report.bootstrap_state,
        report.resource_id,
        report.node_id,
        report.dgraph_id,
        report.services.len(),
        wasm_executed,
        report.execute_requested,
        args.rollback,
        args.cleanup,
        report.control_plane_authorized,
        report.local_container_execution_allowed,
        converged_after,
        report.side_effects_performed,
        report.cleanup.len(),
        cleanup_mutated,
        report.snapshot_published,
        args.out,
    );

    Ok(())
}

fn read_configured_node_id(path: &str) -> Result<Option<String>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {path}"))?;
    let value: toml::Value = toml::from_str(&raw).context("parse operational reconcile TOML config")?;
    Ok(value
        .get("agent")
        .and_then(toml::Value::as_table)
        .and_then(|agent| agent.get("node_id"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string))
}
