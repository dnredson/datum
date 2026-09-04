use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use datum::agent::operational_evidence::{
    append_operational_evidence, publish_operational_evidence, OperationalEvidenceEvent,
};
use datum::agent::service_artifact::{assess_service_artifact, load_service_artifact};
use datum::agent::service_deployment_drift::assess_service_deployment_drift;
use reqwest::blocking::Client;
use serde_json::json;

#[derive(Parser, Debug)]
#[command(name = "smartsentinel-service-evidence")]
#[command(about = "Continuously publish service execution/health/availability and deployment-drift evidence")]
struct Args {
    #[arg(long)]
    artifact: PathBuf,

    #[arg(long)]
    dserver_url: String,

    #[arg(long)]
    project_id: String,

    #[arg(long)]
    resource_id: String,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    dgraph_id: Option<String>,

    #[arg(long)]
    dgraph_revision: Option<u64>,

    #[arg(long, default_value_t = 15)]
    cycle_s: u64,

    #[arg(long)]
    max_cycles: Option<u64>,

    #[arg(long, default_value = "smartsentinel-service-evidence.jsonl")]
    history_out: PathBuf,

    #[arg(long, default_value_t = 5)]
    timeout_s: u64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.cycle_s == 0 {
        anyhow::bail!("--cycle-s must be greater than zero");
    }
    if args.dgraph_revision == Some(0) {
        anyhow::bail!("--dgraph-revision must be greater than zero");
    }
    if args.dgraph_revision.is_some() && args.dgraph_id.is_none() {
        anyhow::bail!("--dgraph-revision requires --dgraph-id");
    }

    let artifact = load_service_artifact(&args.artifact)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(args.timeout_s))
        .build()
        .context("build SmartSentinel service evidence client")?;

    println!(
        "mode=smartsentinel-service-evidence event=start artifact_id={} service_id={} cycle_s={} project_id={} resource_id={} node_id={} dgraph_id={} dgraph_revision={} history={}",
        artifact.artifact_id,
        artifact.service_id,
        args.cycle_s,
        args.project_id,
        args.resource_id,
        args.node_id.as_deref().unwrap_or("none"),
        args.dgraph_id.as_deref().unwrap_or("none"),
        args.dgraph_revision.map(|value| value.to_string()).unwrap_or_else(|| "none".to_string()),
        args.history_out.display()
    );

    let mut sequence = 0u64;
    loop {
        sequence = sequence.saturating_add(1);

        match assess_service_artifact(&artifact) {
            Ok(assessment) => {
                let event = OperationalEvidenceEvent::new(
                    &args.project_id,
                    &args.resource_id,
                    args.node_id.as_deref(),
                    args.dgraph_id.as_deref(),
                    args.dgraph_revision,
                    sequence,
                    "service_operational_assessment",
                    false,
                    serde_json::to_value(&assessment)
                        .unwrap_or_else(|_| json!({"serialization_error":true})),
                )?;
                publish_local_and_remote(&client, &args, &event);
                println!(
                    "mode=smartsentinel-service-evidence event=service_assessment seq={} service_id={} operational_state={} execution={:?} health={:?} availability={:?} side_effects=false",
                    sequence,
                    assessment.service_id,
                    assessment.operational_state,
                    assessment.execution.outcome,
                    assessment.health.outcome,
                    assessment.availability.outcome
                );
            }
            Err(error) => {
                let event = OperationalEvidenceEvent::new(
                    &args.project_id,
                    &args.resource_id,
                    args.node_id.as_deref(),
                    args.dgraph_id.as_deref(),
                    args.dgraph_revision,
                    sequence,
                    "service_operational_assessment_error",
                    false,
                    json!({
                        "artifact_id": artifact.artifact_id,
                        "service_id": artifact.service_id,
                        "error": error.to_string()
                    }),
                )?;
                publish_local_and_remote(&client, &args, &event);
            }
        }

        match assess_service_deployment_drift(&artifact) {
            Ok(drift) => {
                let kind = if drift.drift_detected {
                    "deployment_drift_detected"
                } else {
                    "deployment_converged"
                };
                let event = OperationalEvidenceEvent::new(
                    &args.project_id,
                    &args.resource_id,
                    args.node_id.as_deref(),
                    args.dgraph_id.as_deref(),
                    args.dgraph_revision,
                    sequence,
                    kind,
                    false,
                    serde_json::to_value(&drift)
                        .unwrap_or_else(|_| json!({"serialization_error":true})),
                )?;
                publish_local_and_remote(&client, &args, &event);
                println!(
                    "mode=smartsentinel-service-evidence event=deployment seq={} service_id={} status={} drift_detected={} differences={} side_effects=false",
                    sequence,
                    drift.service_id,
                    drift.status,
                    drift.drift_detected,
                    drift.differences.len()
                );
            }
            Err(error) => {
                let event = OperationalEvidenceEvent::new(
                    &args.project_id,
                    &args.resource_id,
                    args.node_id.as_deref(),
                    args.dgraph_id.as_deref(),
                    args.dgraph_revision,
                    sequence,
                    "deployment_drift_assessment_error",
                    false,
                    json!({
                        "artifact_id": artifact.artifact_id,
                        "service_id": artifact.service_id,
                        "error": error.to_string()
                    }),
                )?;
                publish_local_and_remote(&client, &args, &event);
            }
        }

        if args
            .max_cycles
            .map(|max| sequence >= max)
            .unwrap_or(false)
        {
            break;
        }
        thread::sleep(Duration::from_secs(args.cycle_s));
    }

    println!(
        "mode=smartsentinel-service-evidence event=shutdown completed_cycles={sequence}"
    );
    Ok(())
}

fn publish_local_and_remote(client: &Client, args: &Args, event: &OperationalEvidenceEvent) {
    let local = match append_operational_evidence(&args.history_out, event) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("error:{error}"),
    };
    let remote = match publish_operational_evidence(
        client,
        &args.dserver_url,
        &args.project_id,
        event,
    ) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("error:{error}"),
    };
    println!(
        "mode=smartsentinel-service-evidence event=evidence seq={} kind={} event_id={} local={} remote={} side_effects=false",
        event.sequence, event.event_kind, event.event_id, local, remote
    );
}
