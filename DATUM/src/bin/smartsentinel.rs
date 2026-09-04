use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Result};
use clap::Parser;
use datum::agent::integrated_service_evidence::{
    start_integrated_service_evidence, IntegratedServiceEvidenceOptions,
};
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

    #[arg(long)]
    runtime_artifact: Option<PathBuf>,

    #[arg(long)]
    runtime_execution_user: Option<String>,

    #[arg(long)]
    service_artifact: Option<PathBuf>,

    #[arg(long, default_value_t = 15)]
    service_evidence_s: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.operational {
        let dserver_url = args
            .dserver_url
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--operational requires --dserver-url"))?;
        let project_id = args
            .project_id
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--operational requires --project-id"))?;

        if args.service_artifact.is_some() && args.service_evidence_s == 0 {
            bail!("--service-evidence-s must be greater than zero when --service-artifact is configured");
        }

        let service_observer = if let Some(artifact_path) = args.service_artifact.clone() {
            let shutdown = Arc::new(AtomicBool::new(false));
            let handle = start_integrated_service_evidence(
                IntegratedServiceEvidenceOptions {
                    config_path: args.config.clone(),
                    artifact_path,
                    dserver_url: dserver_url.clone(),
                    project_id: project_id.clone(),
                    timeout_s: args.timeout_s,
                    interval_s: args.service_evidence_s,
                    history_path: args.operational_history_out.clone(),
                    node_id_override: args.node_id.clone(),
                    hostname_override: args.hostname.clone(),
                    stop_file: args.stop_file.clone(),
                },
                Arc::clone(&shutdown),
            )?;
            Some((shutdown, handle))
        } else {
            None
        };

        let result = run_operational_service_loop(OperationalServiceLoopOptions {
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
            runtime_artifact_path: args.runtime_artifact,
            runtime_execution_user: args.runtime_execution_user,
            heartbeat_s: args.heartbeat_s,
            node_id_override: args.node_id,
            hostname_override: args.hostname,
            stop_file: args.stop_file,
            max_cycles: args.max_cycles,
        });

        if let Some((shutdown, handle)) = service_observer {
            shutdown.store(true, Ordering::SeqCst);
            if handle.join().is_err() {
                eprintln!(
                    "mode=smartsentinel-operational event=service_observer_shutdown status=thread_join_error"
                );
            }
        }

        return result;
    }

    if args.dserver_url.is_some()
        || args.project_id.is_some()
        || args.execute
        || args.runtime_artifact.is_some()
        || args.runtime_execution_user.is_some()
        || args.service_artifact.is_some()
    {
        bail!("--dserver-url, --project-id, --execute, --runtime-artifact, --runtime-execution-user, and --service-artifact require --operational");
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
