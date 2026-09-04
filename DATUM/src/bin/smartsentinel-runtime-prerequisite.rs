use std::fs;

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::agent::runtime_prerequisite::reconcile_docker_runtime;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-runtime-prerequisite")]
#[command(about = "Probe or reconcile a local execution runtime prerequisite without arbitrary shell execution")]
struct Args {
    #[arg(long, default_value = "docker")]
    runtime: String,

    #[arg(long, default_value_t = false)]
    execute: bool,

    #[arg(long, default_value = "smartsentinel-runtime-prerequisite.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !args.runtime.trim().eq_ignore_ascii_case("docker") {
        bail!("O2 runtime prerequisite proof currently supports only --runtime docker");
    }

    let report = reconcile_docker_runtime(args.execute)?;
    let json = serde_json::to_string_pretty(&report)
        .context("serialize SmartSentinel runtime prerequisite report")?;
    fs::write(&args.out, format!("{}\n", json))
        .with_context(|| format!("write runtime prerequisite report {}", args.out))?;

    println!(
        "mode=smartsentinel-runtime-prerequisite status=ok runtime={} state={:?} package_manager={} install_supported={} execute_requested={} installation_status={} side_effects={} out={}",
        report.runtime,
        report.before.state,
        report
            .package_manager
            .map(|manager| format!("{:?}", manager))
            .unwrap_or_else(|| "none".to_string()),
        report.install_supported,
        report.execute_requested,
        report.installation_status,
        report.side_effects_performed,
        args.out,
    );

    Ok(())
}
