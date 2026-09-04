use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use datum::agent::runtime_artifact::{
    load_runtime_install_artifact, reconcile_runtime_from_artifact,
};
use datum::agent::runtime_prerequisite::reconcile_docker_runtime;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-runtime-prerequisite")]
#[command(about = "Probe or reconcile a local execution runtime prerequisite without arbitrary shell execution")]
struct Args {
    #[arg(long, default_value = "docker")]
    runtime: String,

    #[arg(long, default_value_t = false)]
    execute: bool,

    #[arg(long)]
    artifact: Option<PathBuf>,

    #[arg(long)]
    execution_user: Option<String>,

    #[arg(long, default_value_t = false)]
    host_mutation_authorized: bool,

    #[arg(long, default_value = "smartsentinel-runtime-prerequisite.json")]
    out: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if !args.runtime.trim().eq_ignore_ascii_case("docker") {
        bail!("O2 runtime prerequisite proof currently supports only --runtime docker");
    }

    if let Some(artifact_path) = args.artifact.as_deref() {
        let artifact = load_runtime_install_artifact(artifact_path)?;
        let execution_authorized = args.execute && args.host_mutation_authorized;
        let report = reconcile_runtime_from_artifact(
            &artifact,
            args.execution_user.as_deref(),
            args.execute,
            execution_authorized,
        )?;
        let json = serde_json::to_string_pretty(&report)
            .context("serialize SmartSentinel runtime artifact report")?;
        fs::write(&args.out, format!("{}\n", json))
            .with_context(|| format!("write runtime artifact report {}", args.out))?;

        println!(
            "mode=smartsentinel-runtime-prerequisite status=ok runtime={} artifact_id={} state={:?} package_manager={} execute_requested={} execution_authorized={} artifact_status={} side_effects={} session_refresh_required={} out={}",
            report.runtime,
            report.artifact_id,
            report.before.state,
            report
                .package_manager
                .map(|manager| format!("{:?}", manager))
                .unwrap_or_else(|| "none".to_string()),
            report.execute_requested,
            report.execution_authorized,
            report.status,
            report.side_effects_performed,
            report.session_refresh_required,
            args.out,
        );
        return Ok(());
    }

    if args.execution_user.is_some() || args.host_mutation_authorized {
        bail!("--execution-user and --host-mutation-authorized require --artifact");
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
