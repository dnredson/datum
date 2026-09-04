use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use datum::agent::service_artifact::{assess_service_artifact, load_service_artifact};

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-service-probe")]
#[command(about = "Assess service execution, health, and availability from a trusted service artifact")]
struct Args {
    #[arg(long)]
    artifact: PathBuf,

    #[arg(long, default_value = "smartsentinel-service-assessment.json")]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let artifact = load_service_artifact(&args.artifact)?;
    let report = assess_service_artifact(&artifact)?;
    let json = serde_json::to_string_pretty(&report)
        .context("serialize SmartSentinel service operational assessment")?;
    if let Some(parent) = args.out.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("create service assessment directory {}", parent.display()))?;
    }
    fs::write(&args.out, format!("{}\n", json))
        .with_context(|| format!("write service assessment {}", args.out.display()))?;

    println!(
        "mode=smartsentinel-service-probe status=ok artifact_id={} service_id={} operational_state={} execution={:?} health={:?} availability={:?} basis={} out={}",
        report.artifact_id,
        report.service_id,
        report.operational_state,
        report.execution.outcome,
        report.health.outcome,
        report.availability.outcome,
        report.assessment_basis,
        args.out.display(),
    );
    Ok(())
}
