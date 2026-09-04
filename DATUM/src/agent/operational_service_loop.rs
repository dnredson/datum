use std::cmp::min;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use fs2::FileExt;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::agent::bootstrap::{run_bootstrap_with_resource_discovery, BootstrapState};
use crate::agent::operational_evidence::{
    append_operational_evidence, publish_operational_evidence, OperationalEvidenceEvent,
};
use crate::agent::operational_reconciliation::{
    publish_operational_role_snapshot, run_operational_reconciliation_with_resource_discovery,
    OperationalReconciliationReport,
};
use crate::agent::resource_discovery::{
    collect_unbound_resource_discovery, derive_resource_lookup_id, load_config_allow_unbound,
    SmartSentinelResourceDiscovery,
};
use crate::config::Config;

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct OperationalServiceLoopOptions {
    pub config_path: String,
    pub dserver_url: String,
    pub project_id: String,
    pub timeout_s: u64,
    pub execute: bool,
    pub report_path: PathBuf,
    pub history_path: PathBuf,
    pub cycle_s_override: Option<u64>,
    pub discovery_refresh_s: u64,
    pub snapshot_s: u64,
    pub drift_verify_s: u64,
    pub heartbeat_s: u64,
    pub node_id_override: Option<String>,
    pub hostname_override: Option<String>,
    pub stop_file: PathBuf,
    pub max_cycles: Option<u64>,
}

impl Default for OperationalServiceLoopOptions {
    fn default() -> Self {
        Self {
            config_path: "config.toml".to_string(),
            dserver_url: String::new(),
            project_id: String::new(),
            timeout_s: 5,
            execute: false,
            report_path: PathBuf::from("smartsentinel-operational.json"),
            history_path: PathBuf::from("smartsentinel-operational-history.jsonl"),
            cycle_s_override: None,
            discovery_refresh_s: 30,
            snapshot_s: 30,
            drift_verify_s: 30,
            heartbeat_s: 1,
            node_id_override: None,
            hostname_override: None,
            stop_file: PathBuf::from(".smartsentinel-stop"),
            max_cycles: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct OperationalControlAuthorizationHead {
    authorization_id: String,
    #[serde(default)]
    dgraph_revision: Option<u64>,
    allow_host_mutation: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct OperationalControlSliceHead {
    dgraph_id: String,
    dgraph_revision: u64,
    node_id: String,
    #[serde(default)]
    authorization: Option<OperationalControlAuthorizationHead>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlSignature {
    dgraph_id: String,
    dgraph_revision: u64,
    authorization_id: Option<String>,
    authorization_revision: Option<u64>,
    allow_host_mutation: bool,
}

impl ControlSignature {
    fn from_head(head: &OperationalControlSliceHead) -> Self {
        Self {
            dgraph_id: head.dgraph_id.clone(),
            dgraph_revision: head.dgraph_revision,
            authorization_id: head
                .authorization
                .as_ref()
                .map(|authorization| authorization.authorization_id.clone()),
            authorization_revision: head
                .authorization
                .as_ref()
                .and_then(|authorization| authorization.dgraph_revision),
            allow_host_mutation: head
                .authorization
                .as_ref()
                .is_some_and(|authorization| authorization.allow_host_mutation),
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "dgraph_id": self.dgraph_id,
            "dgraph_revision": self.dgraph_revision,
            "authorization_id": self.authorization_id,
            "authorization_revision": self.authorization_revision,
            "allow_host_mutation": self.allow_host_mutation
        })
    }
}

pub fn run_operational_service_loop(options: OperationalServiceLoopOptions) -> Result<()> {
    validate_options(&options)?;

    let mut cfg = load_config_allow_unbound(&options.config_path)?;
    if let Some(auto_recovery) = cfg.auto_recovery.as_mut() {
        auto_recovery.enabled = false;
        auto_recovery.dry_run = Some(true);
    }
    if let Some(desired_state) = cfg.desired_state.as_mut() {
        desired_state.enabled = false;
    }

    let configured_node_id = read_configured_node_id(&options.config_path)?;
    let requested_node_id = options
        .node_id_override
        .clone()
        .or(configured_node_id)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let cycle_s = options.cycle_s_override.unwrap_or(cfg.agent.cycle_s);
    if cycle_s == 0 {
        bail!("SmartSentinel operational cycle interval must be greater than zero seconds");
    }

    let mut cached_discovery =
        collect_unbound_resource_discovery(&cfg, options.hostname_override.as_deref())?;
    let resource_id = derive_resource_lookup_id(&cached_discovery);
    let lock_key = operational_lock_key(&resource_id, &cached_discovery.identity.hostname);
    let (_instance_lock, lock_path) = acquire_operational_lock(&lock_key)?;

    let client = Client::builder()
        .timeout(Duration::from_secs(options.timeout_s))
        .build()
        .context("build SmartSentinel operational control-plane client")?;

    let shutdown = Arc::new(AtomicBool::new(false));
    install_ctrlc_handler(Arc::clone(&shutdown))?;

    let heartbeat_sequence = Arc::new(AtomicU64::new(0));
    let started_at = Instant::now();
    let heartbeat_thread = spawn_heartbeat(
        resource_id.clone(),
        requested_node_id.clone(),
        options.stop_file.clone(),
        options.heartbeat_s,
        started_at,
        Arc::clone(&heartbeat_sequence),
        Arc::clone(&shutdown),
    );

    println!(
        "mode=smartsentinel-operational event=start resource_id={} requested_node_id={} cycle_s={} discovery_refresh_s={} snapshot_s={} drift_verify_s={} heartbeat_s={} execute={} dserver_url={} project_id={} lock_file={} history={}",
        resource_id,
        requested_node_id.as_deref().unwrap_or("none"),
        cycle_s,
        options.discovery_refresh_s,
        options.snapshot_s,
        options.drift_verify_s,
        options.heartbeat_s,
        options.execute,
        options.dserver_url.trim_end_matches('/'),
        options.project_id,
        lock_path.display(),
        options.history_path.display(),
    );

    let mut sequence = 0u64;
    let mut last_discovery_refresh = Instant::now();
    let mut last_snapshot: Option<Instant> = None;
    let mut last_drift_verification: Option<Instant> = None;
    let mut last_control_signature: Option<ControlSignature> = None;

    while !shutdown.load(Ordering::SeqCst) {
        detect_stop_file(&options.stop_file, &shutdown);
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        sequence = sequence.saturating_add(1);
        let cycle_started = Instant::now();
        let mut discovery_refreshed = sequence == 1;

        if sequence > 1
            && last_discovery_refresh.elapsed() >= Duration::from_secs(options.discovery_refresh_s)
        {
            match collect_unbound_resource_discovery(&cfg, options.hostname_override.as_deref()) {
                Ok(discovery) => {
                    let observed_resource_id = derive_resource_lookup_id(&discovery);
                    if observed_resource_id != resource_id {
                        record_event(
                            &client,
                            &options,
                            sequence,
                            &resource_id,
                            None,
                            None,
                            None,
                            "identity_changed",
                            false,
                            json!({
                                "status": "identity_changed",
                                "startup_resource_id": resource_id,
                                "observed_resource_id": observed_resource_id,
                                "action": "fail_closed"
                            }),
                        );
                        println!(
                            "mode=smartsentinel-operational event=identity seq={} status=changed startup_resource_id={} observed_resource_id={} action=fail_closed side_effects=false",
                            sequence, resource_id, observed_resource_id
                        );
                        last_discovery_refresh = Instant::now();
                        finish_cycle_wait(&options, &shutdown, cycle_started, cycle_s);
                        continue;
                    }
                    cached_discovery = discovery;
                    discovery_refreshed = true;
                    last_discovery_refresh = Instant::now();
                }
                Err(error) => {
                    println!(
                        "mode=smartsentinel-operational event=resource_discovery seq={} status=refresh_error resource_id={} err={} using_cached=true side_effects=false",
                        sequence, resource_id, error
                    );
                }
            }
        }

        let published_resource_id = match publish_cached_resource_discovery(
            &client,
            &cached_discovery,
            &options.dserver_url,
            &options.project_id,
        ) {
            Ok(published_resource_id) => {
                println!(
                    "mode=smartsentinel-operational event=resource_discovery seq={} status=ok resource_id={} refreshed={}",
                    sequence, published_resource_id, discovery_refreshed
                );
                if discovery_refreshed {
                    record_event(
                        &client,
                        &options,
                        sequence,
                        &resource_id,
                        None,
                        None,
                        None,
                        "resource_discovery",
                        false,
                        serde_json::to_value(&cached_discovery)
                            .unwrap_or_else(|_| json!({"serialization_error":true})),
                    );
                }
                Some(published_resource_id)
            }
            Err(error) => {
                println!(
                    "mode=smartsentinel-operational event=resource_discovery seq={} status=publish_error resource_id={} err={} side_effects=false",
                    sequence, resource_id, error
                );
                record_event(
                    &client,
                    &options,
                    sequence,
                    &resource_id,
                    None,
                    None,
                    None,
                    "resource_discovery_publish_error",
                    false,
                    json!({"error": error.to_string()}),
                );
                None
            }
        };

        if published_resource_id.as_deref() == Some(resource_id.as_str()) {
            run_hybrid_control_cycle(
                &client,
                &cfg,
                &options,
                &cached_discovery,
                requested_node_id.as_deref(),
                sequence,
                cycle_started,
                &resource_id,
                &mut last_snapshot,
                &mut last_drift_verification,
                &mut last_control_signature,
            );
        }

        if options
            .max_cycles
            .map(|max| sequence >= max)
            .unwrap_or(false)
        {
            shutdown.store(true, Ordering::SeqCst);
            break;
        }

        finish_cycle_wait(&options, &shutdown, cycle_started, cycle_s);
    }

    if options.stop_file.exists() {
        fs::remove_file(&options.stop_file).with_context(|| {
            format!(
                "remove SmartSentinel operational stop file {}",
                options.stop_file.display()
            )
        })?;
    }

    if heartbeat_thread.join().is_err() {
        eprintln!("mode=smartsentinel-operational event=liveness status=thread_join_error");
    }

    println!(
        "mode=smartsentinel-operational event=shutdown completed_cycles={} heartbeat_seq={} uptime_ms={}",
        sequence,
        heartbeat_sequence.load(Ordering::Relaxed),
        started_at.elapsed().as_millis(),
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_hybrid_control_cycle(
    client: &Client,
    cfg: &Config,
    options: &OperationalServiceLoopOptions,
    cached_discovery: &SmartSentinelResourceDiscovery,
    requested_node_id: Option<&str>,
    sequence: u64,
    cycle_started: Instant,
    startup_resource_id: &str,
    last_snapshot: &mut Option<Instant>,
    last_drift_verification: &mut Option<Instant>,
    last_control_signature: &mut Option<ControlSignature>,
) {
    let bootstrap = match run_bootstrap_with_resource_discovery(
        cached_discovery.clone(),
        requested_node_id,
        &options.dserver_url,
        &options.project_id,
        options.timeout_s,
    ) {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            record_waiting_or_error(
                client,
                options,
                sequence,
                startup_resource_id,
                requested_node_id,
                error.to_string(),
                cycle_started,
            );
            return;
        }
    };

    if bootstrap.bootstrap_state != BootstrapState::KnownBoundNode {
        *last_control_signature = None;
        record_waiting_or_error(
            client,
            options,
            sequence,
            startup_resource_id,
            requested_node_id,
            format!(
                "operational reconciliation requires known_bound_node, received {:?}",
                bootstrap.bootstrap_state
            ),
            cycle_started,
        );
        return;
    }

    let node_id = match bootstrap.control_plane.resolved_node_id.as_deref() {
        Some(node_id) => node_id,
        None => {
            record_waiting_or_error(
                client,
                options,
                sequence,
                startup_resource_id,
                requested_node_id,
                "known_bound_node bootstrap did not resolve node_id".to_string(),
                cycle_started,
            );
            return;
        }
    };

    let control_head = match fetch_control_slice_head(
        client,
        &options.dserver_url,
        &options.project_id,
        node_id,
    ) {
        Ok(head) => head,
        Err(error) => {
            record_event(
                client,
                options,
                sequence,
                startup_resource_id,
                Some(node_id),
                None,
                None,
                "control_poll_error",
                false,
                json!({"error":error.to_string()}),
            );
            println!(
                "mode=smartsentinel-operational event=control_poll seq={} status=error node_id={} err={} dur_ms={}",
                sequence,
                node_id,
                error,
                cycle_started.elapsed().as_millis()
            );
            return;
        }
    };

    let signature = ControlSignature::from_head(&control_head);
    let control_changed = last_control_signature
        .as_ref()
        .map(|previous| previous != &signature)
        .unwrap_or(true);
    let drift_due = last_drift_verification
        .as_ref()
        .map(|last| last.elapsed() >= Duration::from_secs(options.drift_verify_s))
        .unwrap_or(true);

    if control_changed {
        record_event(
            client,
            options,
            sequence,
            startup_resource_id,
            Some(node_id),
            Some(&control_head.dgraph_id),
            Some(control_head.dgraph_revision),
            "control_change_detected",
            false,
            json!({
                "previous": last_control_signature.as_ref().map(ControlSignature::as_json),
                "current": signature.as_json(),
                "trigger": "event_driven_control_change"
            }),
        );
    }

    if !control_changed && !drift_due {
        let drift_age_ms = last_drift_verification
            .as_ref()
            .map(|last| last.elapsed().as_millis())
            .unwrap_or(0);
        record_event(
            client,
            options,
            sequence,
            startup_resource_id,
            Some(node_id),
            Some(&control_head.dgraph_id),
            Some(control_head.dgraph_revision),
            "reconciliation_skipped",
            false,
            json!({
                "reason": "control_unchanged_and_drift_proof_fresh",
                "trigger": "none",
                "control_signature": signature.as_json(),
                "drift_age_ms": drift_age_ms,
                "drift_verify_s": options.drift_verify_s
            }),
        );
        println!(
            "mode=smartsentinel-operational event=reconcile seq={} status=skipped node_id={} dgraph_id={} dgraph_revision={} reason=control_unchanged_and_drift_proof_fresh drift_age_ms={} dur_ms={}",
            sequence,
            node_id,
            control_head.dgraph_id,
            control_head.dgraph_revision,
            drift_age_ms,
            cycle_started.elapsed().as_millis()
        );
        return;
    }

    let trigger = if control_changed {
        "event_driven_control_change"
    } else {
        "time_driven_drift_verification"
    };

    match run_operational_reconciliation_with_resource_discovery(
        cfg,
        cached_discovery.clone(),
        requested_node_id,
        options.hostname_override.as_deref(),
        &options.dserver_url,
        &options.project_id,
        options.timeout_s,
        options.execute,
        false,
    ) {
        Ok(mut report) => {
            *last_control_signature = Some(signature.clone());
            *last_drift_verification = Some(Instant::now());

            let snapshot_due = report.side_effects_performed
                || last_snapshot
                    .as_ref()
                    .map(|last| last.elapsed() >= Duration::from_secs(options.snapshot_s))
                    .unwrap_or(true);

            if snapshot_due {
                match publish_operational_role_snapshot(
                    cfg,
                    &report.node_id,
                    options.hostname_override.as_deref(),
                    &options.dserver_url,
                    &options.project_id,
                    options.timeout_s,
                ) {
                    Ok(()) => {
                        report.snapshot_published = true;
                        report.snapshot_publish_error = None;
                        *last_snapshot = Some(Instant::now());
                    }
                    Err(error) => {
                        report.snapshot_published = false;
                        report.snapshot_publish_error = Some(error.to_string());
                    }
                }
            }

            write_reconciliation_result(
                client,
                options,
                sequence,
                cycle_started,
                &control_head,
                trigger,
                &report,
                snapshot_due,
            );
        }
        Err(error) => {
            record_event(
                client,
                options,
                sequence,
                startup_resource_id,
                Some(node_id),
                Some(&control_head.dgraph_id),
                Some(control_head.dgraph_revision),
                "reconciliation_error",
                false,
                json!({
                    "error": error.to_string(),
                    "trigger": trigger,
                    "control_signature": signature.as_json()
                }),
            );
            println!(
                "mode=smartsentinel-operational event=reconcile seq={} status=error trigger={} node_id={} dgraph_id={} dgraph_revision={} err={} dur_ms={} side_effects=false",
                sequence,
                trigger,
                node_id,
                control_head.dgraph_id,
                control_head.dgraph_revision,
                error,
                cycle_started.elapsed().as_millis()
            );
        }
    }
}

fn write_reconciliation_result(
    client: &Client,
    options: &OperationalServiceLoopOptions,
    sequence: u64,
    cycle_started: Instant,
    control_head: &OperationalControlSliceHead,
    trigger: &str,
    report: &OperationalReconciliationReport,
    snapshot_due: bool,
) {
    let write_status = match write_report(&options.report_path, report) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("err({error})"),
    };
    let wasm_actions = report
        .services
        .iter()
        .filter(|service| service.wasm.action_code != 0)
        .count();
    let converged_after = report
        .services
        .iter()
        .filter(|service| service.observed_after_running)
        .count();
    let event_kind = if report.side_effects_performed {
        "reconciliation_action"
    } else if trigger == "time_driven_drift_verification" {
        "drift_verification"
    } else {
        "reconciliation"
    };
    let mut event_payload = serde_json::to_value(report)
        .unwrap_or_else(|_| json!({"serialization_error":true}));
    if let Some(object) = event_payload.as_object_mut() {
        object.insert("trigger".to_string(), Value::String(trigger.to_string()));
        object.insert(
            "dgraph_revision".to_string(),
            Value::from(control_head.dgraph_revision),
        );
    }
    record_event(
        client,
        options,
        sequence,
        &report.resource_id,
        Some(&report.node_id),
        Some(&report.dgraph_id),
        Some(control_head.dgraph_revision),
        event_kind,
        report.side_effects_performed,
        event_payload,
    );

    println!(
        "mode=smartsentinel-operational event=reconcile seq={} status=ok trigger={} bootstrap_state={:?} resource_id={} node_id={} dgraph_id={} dgraph_revision={} services={} wasm_actions={} converged_after={} host_mutation={} snapshot_published={} snapshot_due={} report={} report_write={} dur_ms={}",
        sequence,
        trigger,
        report.bootstrap_state,
        report.resource_id,
        report.node_id,
        report.dgraph_id,
        control_head.dgraph_revision,
        report.services.len(),
        wasm_actions,
        converged_after,
        report.side_effects_performed,
        report.snapshot_published,
        snapshot_due,
        options.report_path.display(),
        write_status,
        cycle_started.elapsed().as_millis(),
    );
}

fn fetch_control_slice_head(
    client: &Client,
    dserver_url: &str,
    project_id: &str,
    node_id: &str,
) -> Result<OperationalControlSliceHead> {
    let url = format!(
        "{}/api/v1/nodes/{}/operational-dgraph-slice",
        dserver_url.trim().trim_end_matches('/'),
        node_id.trim()
    );
    let response = client
        .get(&url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("poll operational D-Graph control slice {url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!("operational control slice poll failed with status {status}: {body}");
    }
    let head: OperationalControlSliceHead = response
        .json()
        .context("decode operational D-Graph control slice head")?;
    if head.dgraph_revision == 0 {
        bail!("operational control slice returned revision zero");
    }
    if head.node_id.trim() != node_id.trim() {
        bail!(
            "operational control slice node '{}' does not match resolved node '{}'",
            head.node_id,
            node_id
        );
    }
    Ok(head)
}

fn record_waiting_or_error(
    client: &Client,
    options: &OperationalServiceLoopOptions,
    sequence: u64,
    startup_resource_id: &str,
    requested_node_id: Option<&str>,
    message: String,
    cycle_started: Instant,
) {
    let waiting = message.contains("KnownUnboundResource")
        || message.contains("UnknownResource")
        || message.contains("known_unbound_resource")
        || message.contains("unknown_resource");
    record_event(
        client,
        options,
        sequence,
        startup_resource_id,
        None,
        None,
        None,
        if waiting {
            "reconciliation_waiting"
        } else {
            "reconciliation_error"
        },
        false,
        json!({
            "error": message,
            "requested_node_id": requested_node_id,
            "waiting_for_binding": waiting
        }),
    );
    println!(
        "mode=smartsentinel-operational event=reconcile seq={} status=waiting_or_error resource_id={} requested_node_id={} err={} dur_ms={} side_effects=false",
        sequence,
        startup_resource_id,
        requested_node_id.unwrap_or("none"),
        message,
        cycle_started.elapsed().as_millis(),
    );
}

#[allow(clippy::too_many_arguments)]
fn record_event(
    client: &Client,
    options: &OperationalServiceLoopOptions,
    sequence: u64,
    resource_id: &str,
    node_id: Option<&str>,
    dgraph_id: Option<&str>,
    dgraph_revision: Option<u64>,
    event_kind: &str,
    side_effects_performed: bool,
    payload: Value,
) {
    let event = match OperationalEvidenceEvent::new(
        &options.project_id,
        resource_id,
        node_id,
        dgraph_id,
        dgraph_revision,
        sequence,
        event_kind,
        side_effects_performed,
        payload,
    ) {
        Ok(event) => event,
        Err(error) => {
            eprintln!(
                "mode=smartsentinel-operational event=evidence seq={} status=build_error kind={} err={}",
                sequence, event_kind, error
            );
            return;
        }
    };

    let local_status = match append_operational_evidence(&options.history_path, &event) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("err({error})"),
    };
    let remote_status = match publish_operational_evidence(
        client,
        &options.dserver_url,
        &options.project_id,
        &event,
    ) {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("err({error})"),
    };
    println!(
        "mode=smartsentinel-operational event=evidence seq={} kind={} event_id={} local={} remote={} side_effects={} dgraph_revision={}",
        sequence,
        event_kind,
        event.event_id,
        local_status,
        remote_status,
        event.side_effects_performed,
        event
            .dgraph_revision
            .map(|revision| revision.to_string())
            .unwrap_or_else(|| "none".to_string()),
    );
}

fn publish_cached_resource_discovery(
    client: &Client,
    discovery: &SmartSentinelResourceDiscovery,
    dserver_url: &str,
    project_id: &str,
) -> Result<String> {
    let resource_id = derive_resource_lookup_id(discovery);
    let url = format!(
        "{}/api/v1/resources/discovery",
        dserver_url.trim().trim_end_matches('/')
    );
    let response = client
        .post(&url)
        .header("X-Datum-Project", project_id.trim())
        .json(discovery)
        .send()
        .with_context(|| format!("publish SmartSentinel cached role-free discovery {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("dserver resource discovery publish failed with status {status}: {body}");
    }
    let body: Value = response
        .json()
        .context("decode dserver resource discovery response")?;
    let response_resource_id = body
        .get("resource_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("dserver resource discovery response omitted resource_id"))?;
    if response_resource_id != resource_id {
        bail!(
            "dserver resource discovery returned resource_id '{}' but cached local discovery derived '{}'",
            response_resource_id,
            resource_id
        );
    }
    Ok(resource_id)
}

fn validate_options(options: &OperationalServiceLoopOptions) -> Result<()> {
    if options.dserver_url.trim().is_empty() {
        bail!("SmartSentinel operational mode requires non-empty dserver_url");
    }
    if options.project_id.trim().is_empty() {
        bail!("SmartSentinel operational mode requires non-empty project_id");
    }
    if options.timeout_s == 0 {
        bail!("SmartSentinel operational timeout must be greater than zero seconds");
    }
    if options.discovery_refresh_s == 0 {
        bail!("SmartSentinel operational discovery refresh interval must be greater than zero seconds");
    }
    if options.snapshot_s == 0 {
        bail!("SmartSentinel operational snapshot interval must be greater than zero seconds");
    }
    if options.drift_verify_s == 0 {
        bail!("SmartSentinel operational drift verification interval must be greater than zero seconds");
    }
    if options.heartbeat_s == 0 {
        bail!("SmartSentinel operational heartbeat interval must be greater than zero seconds");
    }
    if options.max_cycles == Some(0) {
        bail!("SmartSentinel operational max-cycles must be greater than zero when provided");
    }
    if options.stop_file.exists() {
        bail!(
            "SmartSentinel stop file already exists: {}. Remove it before starting the agent.",
            options.stop_file.display()
        );
    }
    Ok(())
}

fn operational_lock_key(resource_id: &str, hostname: &str) -> String {
    if resource_id.starts_with("resource:instance:") {
        format!("operational-host:{}", hostname.trim())
    } else {
        format!("operational:{}", resource_id.trim())
    }
}

fn acquire_operational_lock(lock_key: &str) -> Result<(File, PathBuf)> {
    let safe = crate::core::ids::sanitize_id_component(lock_key);
    let path = std::env::temp_dir().join(format!("smartsentinel-{}.lock", safe));
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open SmartSentinel operational lock {}", path.display()))?;

    if let Err(error) = file.try_lock_exclusive() {
        bail!(
            "SmartSentinel operational instance already running for lock_key={} (lock={}): {}",
            lock_key,
            path.display(),
            error
        );
    }

    file.set_len(0)
        .with_context(|| format!("truncate SmartSentinel operational lock {}", path.display()))?;
    write!(file, "pid={}\nlock_key={}\n", std::process::id(), lock_key)
        .with_context(|| format!("write SmartSentinel operational lock {}", path.display()))?;
    file.flush()
        .with_context(|| format!("flush SmartSentinel operational lock {}", path.display()))?;

    Ok((file, path))
}

fn read_configured_node_id(path: &str) -> Result<Option<String>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {path}"))?;
    let value: toml::Value = toml::from_str(&raw).context("parse operational service TOML config")?;
    Ok(value
        .get("agent")
        .and_then(toml::Value::as_table)
        .and_then(|agent| agent.get("node_id"))
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string))
}

fn write_report(path: &Path, report: &OperationalReconciliationReport) -> Result<()> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("create operational report directory {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(report)
        .context("serialize continuous operational reconciliation report")?;
    fs::write(path, format!("{}\n", json))
        .with_context(|| format!("write operational report {}", path.display()))
}

fn install_ctrlc_handler(shutdown: Arc<AtomicBool>) -> Result<()> {
    ctrlc::set_handler(move || {
        shutdown.store(true, Ordering::SeqCst);
    })
    .context("install SmartSentinel operational Ctrl+C handler")
}

fn spawn_heartbeat(
    resource_id: String,
    requested_node_id: Option<String>,
    stop_file: PathBuf,
    heartbeat_s: u64,
    started_at: Instant,
    heartbeat_sequence: Arc<AtomicU64>,
    shutdown: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let interval = Duration::from_secs(heartbeat_s);
        while !shutdown.load(Ordering::SeqCst) {
            detect_stop_file(&stop_file, &shutdown);
            if shutdown.load(Ordering::SeqCst) {
                break;
            }
            let sequence = heartbeat_sequence.fetch_add(1, Ordering::Relaxed) + 1;
            println!(
                "mode=smartsentinel-operational event=liveness seq={} resource_id={} requested_node_id={} status=alive uptime_ms={}",
                sequence,
                resource_id,
                requested_node_id.as_deref().unwrap_or("none"),
                started_at.elapsed().as_millis(),
            );
            wait_with_shutdown(&stop_file, &shutdown, interval);
        }
    })
}

fn detect_stop_file(stop_file: &Path, shutdown: &AtomicBool) {
    if stop_file.exists() {
        shutdown.store(true, Ordering::SeqCst);
    }
}

fn wait_with_shutdown(stop_file: &Path, shutdown: &AtomicBool, duration: Duration) {
    let started = Instant::now();
    while started.elapsed() < duration {
        detect_stop_file(stop_file, shutdown);
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        let remaining = duration.saturating_sub(started.elapsed());
        thread::sleep(min(STOP_POLL_INTERVAL, remaining));
    }
}

fn finish_cycle_wait(
    options: &OperationalServiceLoopOptions,
    shutdown: &AtomicBool,
    cycle_started: Instant,
    cycle_s: u64,
) {
    let elapsed = cycle_started.elapsed();
    let target = Duration::from_secs(cycle_s);
    if elapsed < target {
        wait_with_shutdown(&options.stop_file, shutdown, target - elapsed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_resource_identity_is_used_for_operational_lock() {
        assert_eq!(
            operational_lock_key("resource:os:abc", "duplicate-hostname"),
            "operational:resource:os:abc"
        );
    }

    #[test]
    fn ephemeral_resource_identity_uses_hostname_only_for_local_mutex() {
        assert_eq!(
            operational_lock_key("resource:instance:random", "Xubuntu"),
            "operational-host:Xubuntu"
        );
    }

    #[test]
    fn operational_options_reject_missing_control_plane_context() {
        let mut options = OperationalServiceLoopOptions::default();
        assert!(validate_options(&options).is_err());
        options.dserver_url = "http://127.0.0.1:8080".to_string();
        options.project_id = "project-a".to_string();
        assert!(validate_options(&options).is_ok());
    }

    #[test]
    fn control_signature_changes_on_revision_or_authorization() {
        let base = OperationalControlSliceHead {
            dgraph_id: "dgraph:a".to_string(),
            dgraph_revision: 1,
            node_id: "fog-01".to_string(),
            authorization: None,
        };
        let first = ControlSignature::from_head(&base);

        let mut revised = base.clone();
        revised.dgraph_revision = 2;
        assert_ne!(first, ControlSignature::from_head(&revised));

        let mut authorized = base;
        authorized.authorization = Some(OperationalControlAuthorizationHead {
            authorization_id: "opauth:1".to_string(),
            dgraph_revision: Some(1),
            allow_host_mutation: true,
        });
        assert_ne!(first, ControlSignature::from_head(&authorized));
    }
}
