use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::bootstrap::{run_bootstrap_with_resource_discovery, BootstrapState};
use crate::agent::operational_evidence::{
    append_operational_evidence, publish_operational_evidence, OperationalEvidenceEvent,
};
use crate::agent::operational_reconciliation::publish_operational_role_snapshot;
use crate::agent::resource_discovery::{
    collect_unbound_resource_discovery, derive_resource_lookup_id, load_config_allow_unbound,
    SmartSentinelResourceDiscovery,
};
use crate::agent::service_artifact::{
    assess_service_artifact, load_service_artifact, ServiceArtifact,
};
use crate::agent::service_deployment_drift::assess_service_deployment_drift;
use crate::agent::service_deployment_reconciliation::reconcile_service_deployment_from_artifact;
use crate::config::Config;

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct IntegratedServiceEvidenceOptions {
    pub config_path: String,
    pub artifact_path: PathBuf,
    pub dserver_url: String,
    pub project_id: String,
    pub timeout_s: u64,
    pub interval_s: u64,
    pub history_path: PathBuf,
    pub node_id_override: Option<String>,
    pub hostname_override: Option<String>,
    pub stop_file: PathBuf,
    pub execute: bool,
}

#[derive(Debug, Clone, Default)]
struct ServiceEvidenceControlContext {
    node_id: Option<String>,
    dgraph_id: Option<String>,
    dgraph_revision: Option<u64>,
    desired_in_active_slice: Option<bool>,
    control_plane_authorized: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct OperationalSliceServiceHead {
    service_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OperationalSliceAuthorizationHead {
    #[serde(default)]
    dgraph_revision: Option<u64>,
    allow_host_mutation: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct OperationalSliceContextHead {
    dgraph_id: String,
    dgraph_revision: u64,
    node_id: String,
    #[serde(default)]
    desired_services: Vec<OperationalSliceServiceHead>,
    #[serde(default)]
    authorization: Option<OperationalSliceAuthorizationHead>,
}

pub fn start_integrated_service_evidence(
    options: IntegratedServiceEvidenceOptions,
    shutdown: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>> {
    validate_options(&options)?;

    let artifact = load_service_artifact(&options.artifact_path)?;
    let cfg = load_config_allow_unbound(&options.config_path)?;
    let discovery = collect_unbound_resource_discovery(&cfg, options.hostname_override.as_deref())?;
    let resource_id = derive_resource_lookup_id(&discovery);
    let requested_node_id = options
        .node_id_override
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let local_container_execution_allowed = cfg.dispatch.as_ref().is_some_and(|dispatch| {
        dispatch.enabled && dispatch.allow_container_execution.unwrap_or(false)
    });
    let client = Client::builder()
        .timeout(Duration::from_secs(options.timeout_s))
        .build()
        .context("build integrated SmartSentinel service evidence client")?;

    println!(
        "mode=smartsentinel-operational event=service_observer_start artifact_id={} service_id={} interval_s={} resource_id={} execute={} local_container_execution_allowed={} history={} side_effects=false",
        artifact.artifact_id,
        artifact.service_id,
        options.interval_s,
        resource_id,
        options.execute,
        local_container_execution_allowed,
        options.history_path.display(),
    );

    Ok(thread::spawn(move || {
        run_service_evidence_loop(
            client,
            options,
            cfg,
            artifact,
            discovery,
            resource_id,
            requested_node_id,
            local_container_execution_allowed,
            shutdown,
        );
    }))
}

#[allow(clippy::too_many_arguments)]
fn run_service_evidence_loop(
    client: Client,
    options: IntegratedServiceEvidenceOptions,
    cfg: Config,
    artifact: ServiceArtifact,
    discovery: SmartSentinelResourceDiscovery,
    resource_id: String,
    requested_node_id: Option<String>,
    local_container_execution_allowed: bool,
    shutdown: Arc<AtomicBool>,
) {
    let interval = Duration::from_secs(options.interval_s);
    let mut sequence = 0u64;

    while !shutdown.load(Ordering::SeqCst) && !options.stop_file.exists() {
        sequence = sequence.saturating_add(1);
        let cycle_started = Instant::now();
        let context = resolve_control_context(
            &client,
            &options,
            &artifact,
            &discovery,
            requested_node_id.as_deref(),
            sequence,
        );

        match assess_service_artifact(&artifact) {
            Ok(assessment) => {
                let mut payload = serde_json::to_value(&assessment)
                    .unwrap_or_else(|_| json!({"serialization_error":true}));
                add_integration_context(
                    &mut payload,
                    &context,
                    options.execute,
                    local_container_execution_allowed,
                );
                record_event(
                    &client,
                    &options,
                    sequence,
                    &resource_id,
                    &context,
                    "service_operational_assessment",
                    false,
                    payload,
                );
                println!(
                    "mode=smartsentinel-operational event=service_assessment seq={} service_id={} operational_state={} execution={:?} health={:?} availability={:?} desired_in_active_slice={} control_plane_authorized={} side_effects=false",
                    sequence,
                    assessment.service_id,
                    assessment.operational_state,
                    assessment.execution.outcome,
                    assessment.health.outcome,
                    assessment.availability.outcome,
                    optional_bool_text(context.desired_in_active_slice),
                    context.control_plane_authorized,
                );
            }
            Err(error) => {
                record_event(
                    &client,
                    &options,
                    sequence,
                    &resource_id,
                    &context,
                    "service_operational_assessment_error",
                    false,
                    json!({
                        "artifact_id": artifact.artifact_id.clone(),
                        "service_id": artifact.service_id.clone(),
                        "error": error.to_string(),
                        "desired_in_active_slice": context.desired_in_active_slice,
                        "control_plane_authorized": context.control_plane_authorized,
                        "execute_requested": options.execute,
                        "local_container_execution_allowed": local_container_execution_allowed,
                        "integration_source": "smartsentinel_operational"
                    }),
                );
            }
        }

        match assess_service_deployment_drift(&artifact) {
            Ok(drift) => {
                let event_kind = if drift.drift_detected {
                    "deployment_drift_detected"
                } else {
                    "deployment_converged"
                };
                let mut payload = serde_json::to_value(&drift)
                    .unwrap_or_else(|_| json!({"serialization_error":true}));
                add_integration_context(
                    &mut payload,
                    &context,
                    options.execute,
                    local_container_execution_allowed,
                );
                record_event(
                    &client,
                    &options,
                    sequence,
                    &resource_id,
                    &context,
                    event_kind,
                    false,
                    payload,
                );
                println!(
                    "mode=smartsentinel-operational event=service_deployment seq={} service_id={} status={} drift_detected={} differences={} desired_in_active_slice={} control_plane_authorized={} side_effects=false",
                    sequence,
                    drift.service_id,
                    drift.status,
                    drift.drift_detected,
                    drift.differences.len(),
                    optional_bool_text(context.desired_in_active_slice),
                    context.control_plane_authorized,
                );

                if drift.drift_detected {
                    reconcile_detected_deployment_drift(
                        &client,
                        &options,
                        &cfg,
                        &artifact,
                        sequence,
                        &resource_id,
                        &context,
                        local_container_execution_allowed,
                    );
                }
            }
            Err(error) => {
                record_event(
                    &client,
                    &options,
                    sequence,
                    &resource_id,
                    &context,
                    "deployment_drift_assessment_error",
                    false,
                    json!({
                        "artifact_id": artifact.artifact_id.clone(),
                        "service_id": artifact.service_id.clone(),
                        "error": error.to_string(),
                        "desired_in_active_slice": context.desired_in_active_slice,
                        "control_plane_authorized": context.control_plane_authorized,
                        "execute_requested": options.execute,
                        "local_container_execution_allowed": local_container_execution_allowed,
                        "integration_source": "smartsentinel_operational"
                    }),
                );
            }
        }

        wait_with_shutdown(
            &options.stop_file,
            &shutdown,
            interval.saturating_sub(cycle_started.elapsed()),
        );
    }

    println!(
        "mode=smartsentinel-operational event=service_observer_shutdown completed_cycles={} artifact_id={} service_id={}",
        sequence, artifact.artifact_id, artifact.service_id
    );
}

#[allow(clippy::too_many_arguments)]
fn reconcile_detected_deployment_drift(
    client: &Client,
    options: &IntegratedServiceEvidenceOptions,
    cfg: &Config,
    artifact: &ServiceArtifact,
    sequence: u64,
    resource_id: &str,
    context: &ServiceEvidenceControlContext,
    local_container_execution_allowed: bool,
) {
    let Some(desired_in_active_slice) = context.desired_in_active_slice else {
        record_event(
            client,
            options,
            sequence,
            resource_id,
            context,
            "service_deployment_reconciliation_blocked",
            false,
            json!({
                "artifact_id": artifact.artifact_id.clone(),
                "service_id": artifact.service_id.clone(),
                "status": "blocked_control_context_unknown",
                "execute_requested": options.execute,
                "desired_in_active_slice": Value::Null,
                "control_plane_authorized": false,
                "local_container_execution_allowed": local_container_execution_allowed,
                "side_effects_performed": false,
                "integration_source": "smartsentinel_operational"
            }),
        );
        return;
    };

    match reconcile_service_deployment_from_artifact(
        artifact,
        options.execute,
        desired_in_active_slice,
        context.control_plane_authorized,
        local_container_execution_allowed,
    ) {
        Ok(report) => {
            let event_kind = if report.side_effects_performed {
                "service_deployment_reconciliation_action"
            } else if report.status.starts_with("blocked_") {
                "service_deployment_reconciliation_blocked"
            } else {
                "service_deployment_reconciliation"
            };

            let mut snapshot_published = false;
            let mut snapshot_publish_error: Option<String> = None;
            if report.side_effects_performed {
                if let Some(node_id) = context.node_id.as_deref() {
                    match publish_operational_role_snapshot(
                        cfg,
                        node_id,
                        options.hostname_override.as_deref(),
                        &options.dserver_url,
                        &options.project_id,
                        options.timeout_s,
                    ) {
                        Ok(()) => snapshot_published = true,
                        Err(error) => snapshot_publish_error = Some(error.to_string()),
                    }
                } else {
                    snapshot_publish_error = Some(
                        "deployment action completed but no resolved node_id was available for snapshot"
                            .to_string(),
                    );
                }
            }

            let mut payload = serde_json::to_value(&report)
                .unwrap_or_else(|_| json!({"serialization_error":true}));
            add_integration_context(
                &mut payload,
                context,
                options.execute,
                local_container_execution_allowed,
            );
            if let Some(object) = payload.as_object_mut() {
                object.insert(
                    "snapshot_published".to_string(),
                    Value::Bool(snapshot_published),
                );
                object.insert(
                    "snapshot_publish_error".to_string(),
                    snapshot_publish_error
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
            }
            record_event(
                client,
                options,
                sequence,
                resource_id,
                context,
                event_kind,
                report.side_effects_performed,
                payload,
            );
            println!(
                "mode=smartsentinel-operational event=service_deployment_reconcile seq={} service_id={} status={} execute_requested={} desired_in_active_slice={} control_plane_authorized={} local_container_execution_allowed={} side_effects={} snapshot_published={} command_count={}",
                sequence,
                report.service_id,
                report.status,
                report.execute_requested,
                report.desired_in_active_slice,
                report.control_plane_authorized,
                report.local_container_execution_allowed,
                report.side_effects_performed,
                snapshot_published,
                report.command_results.len(),
            );
        }
        Err(error) => {
            record_event(
                client,
                options,
                sequence,
                resource_id,
                context,
                "service_deployment_reconciliation_error",
                false,
                json!({
                    "artifact_id": artifact.artifact_id.clone(),
                    "service_id": artifact.service_id.clone(),
                    "error": error.to_string(),
                    "execute_requested": options.execute,
                    "desired_in_active_slice": desired_in_active_slice,
                    "control_plane_authorized": context.control_plane_authorized,
                    "local_container_execution_allowed": local_container_execution_allowed,
                    "integration_source": "smartsentinel_operational"
                }),
            );
        }
    }
}

fn resolve_control_context(
    client: &Client,
    options: &IntegratedServiceEvidenceOptions,
    artifact: &ServiceArtifact,
    discovery: &SmartSentinelResourceDiscovery,
    requested_node_id: Option<&str>,
    sequence: u64,
) -> ServiceEvidenceControlContext {
    let bootstrap = match run_bootstrap_with_resource_discovery(
        discovery.clone(),
        requested_node_id,
        &options.dserver_url,
        &options.project_id,
        options.timeout_s,
    ) {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            eprintln!(
                "mode=smartsentinel-operational event=service_context seq={} status=bootstrap_error err={} side_effects=false",
                sequence, error
            );
            return ServiceEvidenceControlContext::default();
        }
    };

    if bootstrap.bootstrap_state != BootstrapState::KnownBoundNode {
        println!(
            "mode=smartsentinel-operational event=service_context seq={} status={:?} resource_id={} side_effects=false",
            sequence,
            bootstrap.bootstrap_state,
            bootstrap.control_plane.resource_id,
        );
        return ServiceEvidenceControlContext::default();
    }

    let Some(node_id) = bootstrap.control_plane.resolved_node_id else {
        eprintln!(
            "mode=smartsentinel-operational event=service_context seq={} status=missing_resolved_node side_effects=false",
            sequence
        );
        return ServiceEvidenceControlContext::default();
    };

    match fetch_slice_context(client, options, &node_id) {
        Ok(slice) => {
            let control_plane_authorized = slice.authorization.as_ref().is_some_and(|authorization| {
                authorization.allow_host_mutation
                    && authorization.dgraph_revision == Some(slice.dgraph_revision)
            });
            ServiceEvidenceControlContext {
                desired_in_active_slice: Some(
                    slice
                        .desired_services
                        .iter()
                        .any(|service| service.service_id.trim() == artifact.service_id.trim()),
                ),
                node_id: Some(node_id),
                dgraph_id: Some(slice.dgraph_id),
                dgraph_revision: Some(slice.dgraph_revision),
                control_plane_authorized,
            }
        }
        Err(error) => {
            eprintln!(
                "mode=smartsentinel-operational event=service_context seq={} status=slice_error node_id={} err={} side_effects=false",
                sequence, node_id, error
            );
            ServiceEvidenceControlContext {
                node_id: Some(node_id),
                ..ServiceEvidenceControlContext::default()
            }
        }
    }
}

fn fetch_slice_context(
    client: &Client,
    options: &IntegratedServiceEvidenceOptions,
    node_id: &str,
) -> Result<OperationalSliceContextHead> {
    let url = format!(
        "{}/api/v1/nodes/{}/operational-dgraph-slice",
        options.dserver_url.trim().trim_end_matches('/'),
        node_id.trim()
    );
    let response = client
        .get(&url)
        .header("X-Datum-Project", options.project_id.trim())
        .send()
        .with_context(|| format!("fetch integrated service D-Graph context {url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!("integrated service D-Graph context failed with status {status}: {body}");
    }
    let slice: OperationalSliceContextHead = response
        .json()
        .context("decode integrated service operational D-Graph slice")?;
    if slice.dgraph_revision == 0 {
        bail!("integrated service D-Graph context returned revision zero");
    }
    if slice.node_id.trim() != node_id.trim() {
        bail!(
            "integrated service D-Graph node '{}' does not match resolved node '{}'",
            slice.node_id,
            node_id
        );
    }
    Ok(slice)
}

fn add_integration_context(
    payload: &mut Value,
    context: &ServiceEvidenceControlContext,
    execute_requested: bool,
    local_container_execution_allowed: bool,
) {
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "integration_source".to_string(),
            Value::String("smartsentinel_operational".to_string()),
        );
        object.insert(
            "desired_in_active_slice".to_string(),
            context
                .desired_in_active_slice
                .map(Value::Bool)
                .unwrap_or(Value::Null),
        );
        object.insert(
            "control_plane_authorized".to_string(),
            Value::Bool(context.control_plane_authorized),
        );
        object.insert(
            "execute_requested".to_string(),
            Value::Bool(execute_requested),
        );
        object.insert(
            "local_container_execution_allowed".to_string(),
            Value::Bool(local_container_execution_allowed),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn record_event(
    client: &Client,
    options: &IntegratedServiceEvidenceOptions,
    sequence: u64,
    resource_id: &str,
    context: &ServiceEvidenceControlContext,
    event_kind: &str,
    side_effects_performed: bool,
    payload: Value,
) {
    let event = match OperationalEvidenceEvent::new(
        &options.project_id,
        resource_id,
        context.node_id.as_deref(),
        context.dgraph_id.as_deref(),
        context.dgraph_revision,
        sequence,
        event_kind,
        side_effects_performed,
        payload,
    ) {
        Ok(event) => event,
        Err(error) => {
            eprintln!(
                "mode=smartsentinel-operational event=service_evidence seq={} kind={} status=build_error err={}",
                sequence, event_kind, error
            );
            return;
        }
    };

    let local = match append_operational_evidence(&options.history_path, &event) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("err({error})"),
    };
    let remote = match publish_operational_evidence(
        client,
        &options.dserver_url,
        &options.project_id,
        &event,
    ) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("err({error})"),
    };
    println!(
        "mode=smartsentinel-operational event=service_evidence seq={} kind={} event_id={} local={} remote={} dgraph_revision={} side_effects={}",
        sequence,
        event_kind,
        event.event_id,
        local,
        remote,
        event
            .dgraph_revision
            .map(|revision| revision.to_string())
            .unwrap_or_else(|| "none".to_string()),
        event.side_effects_performed,
    );
}

fn validate_options(options: &IntegratedServiceEvidenceOptions) -> Result<()> {
    if options.config_path.trim().is_empty() {
        bail!("integrated service evidence config_path must not be empty");
    }
    if options.dserver_url.trim().is_empty() {
        bail!("integrated service evidence dserver_url must not be empty");
    }
    if options.project_id.trim().is_empty() {
        bail!("integrated service evidence project_id must not be empty");
    }
    if options.timeout_s == 0 {
        bail!("integrated service evidence timeout must be greater than zero seconds");
    }
    if options.interval_s == 0 {
        bail!("integrated service evidence interval must be greater than zero seconds");
    }
    Ok(())
}

fn wait_with_shutdown(stop_file: &std::path::Path, shutdown: &AtomicBool, duration: Duration) {
    let started = Instant::now();
    while started.elapsed() < duration {
        if shutdown.load(Ordering::SeqCst) || stop_file.exists() {
            return;
        }
        let remaining = duration.saturating_sub(started.elapsed());
        thread::sleep(std::cmp::min(STOP_POLL_INTERVAL, remaining));
    }
}

fn optional_bool_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}
