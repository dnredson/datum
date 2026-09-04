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
use serde_json::Value;

use crate::agent::operational_reconciliation::run_operational_reconciliation;
use crate::agent::resource_discovery::{
    collect_unbound_resource_discovery, derive_resource_lookup_id, load_config_allow_unbound,
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
    pub cycle_s_override: Option<u64>,
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
            cycle_s_override: None,
            heartbeat_s: 1,
            node_id_override: None,
            hostname_override: None,
            stop_file: PathBuf::from(".smartsentinel-stop"),
            max_cycles: None,
        }
    }
}

pub fn run_operational_service_loop(options: OperationalServiceLoopOptions) -> Result<()> {
    validate_options(&options)?;

    let mut cfg = load_config_allow_unbound(&options.config_path)?;

    // The operational path owns desired-state reconciliation. Legacy auto-recovery and
    // desired-state mechanisms remain disabled so there is only one mutation path.
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

    let initial_discovery =
        collect_unbound_resource_discovery(&cfg, options.hostname_override.as_deref())?;
    let resource_id = derive_resource_lookup_id(&initial_discovery);
    let lock_key = operational_lock_key(&resource_id, &initial_discovery.identity.hostname);
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
        "mode=smartsentinel-operational event=start resource_id={} requested_node_id={} cycle_s={} heartbeat_s={} execute={} dserver_url={} project_id={} lock_file={}",
        resource_id,
        requested_node_id.as_deref().unwrap_or("none"),
        cycle_s,
        options.heartbeat_s,
        options.execute,
        options.dserver_url.trim_end_matches('/'),
        options.project_id,
        lock_path.display(),
    );

    let mut sequence = 0u64;
    while !shutdown.load(Ordering::SeqCst) {
        detect_stop_file(&options.stop_file, &shutdown);
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        sequence = sequence.saturating_add(1);
        let cycle_started = Instant::now();

        let published_resource_id = match publish_resource_discovery(
            &client,
            &cfg,
            options.hostname_override.as_deref(),
            &options.dserver_url,
            &options.project_id,
        ) {
            Ok(published_resource_id) => {
                println!(
                    "mode=smartsentinel-operational event=resource_discovery seq={} status=ok resource_id={}",
                    sequence, published_resource_id
                );
                Some(published_resource_id)
            }
            Err(error) => {
                println!(
                    "mode=smartsentinel-operational event=resource_discovery seq={} status=publish_error resource_id={} err={} side_effects=false",
                    sequence, resource_id, error
                );
                None
            }
        };

        if let Some(published_resource_id) = published_resource_id {
            if published_resource_id != resource_id {
                println!(
                    "mode=smartsentinel-operational event=reconcile seq={} status=identity_changed startup_resource_id={} observed_resource_id={} action=fail_closed side_effects=false",
                    sequence, resource_id, published_resource_id
                );
            } else {
                run_reconcile_cycle(
                    &cfg,
                    &options,
                    requested_node_id.as_deref(),
                    sequence,
                    cycle_started,
                    &resource_id,
                );
            }
        }

        if options
            .max_cycles
            .map(|max| sequence >= max)
            .unwrap_or(false)
        {
            shutdown.store(true, Ordering::SeqCst);
            break;
        }

        wait_with_shutdown(
            &options.stop_file,
            &shutdown,
            Duration::from_secs(cycle_s),
        );
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

fn run_reconcile_cycle(
    cfg: &Config,
    options: &OperationalServiceLoopOptions,
    requested_node_id: Option<&str>,
    sequence: u64,
    cycle_started: Instant,
    startup_resource_id: &str,
) {
    match run_operational_reconciliation(
        cfg,
        requested_node_id,
        options.hostname_override.as_deref(),
        &options.dserver_url,
        &options.project_id,
        options.timeout_s,
        options.execute,
    ) {
        Ok(report) => {
            let write_status = match write_report(&options.report_path, &report) {
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

            println!(
                "mode=smartsentinel-operational event=reconcile seq={} status=ok bootstrap_state={:?} resource_id={} node_id={} dgraph_id={} services={} wasm_actions={} converged_after={} host_mutation={} snapshot_published={} report={} report_write={} dur_ms={}",
                sequence,
                report.bootstrap_state,
                report.resource_id,
                report.node_id,
                report.dgraph_id,
                report.services.len(),
                wasm_actions,
                converged_after,
                report.side_effects_performed,
                report.snapshot_published,
                options.report_path.display(),
                write_status,
                cycle_started.elapsed().as_millis(),
            );
        }
        Err(error) => {
            // Starting before registration/binding is expected. Keep the process alive and
            // retry so a resource can receive its role after SmartSentinel has already started.
            println!(
                "mode=smartsentinel-operational event=reconcile seq={} status=waiting_or_error resource_id={} requested_node_id={} err={} dur_ms={} side_effects=false",
                sequence,
                startup_resource_id,
                requested_node_id.unwrap_or("none"),
                error,
                cycle_started.elapsed().as_millis(),
            );
        }
    }
}

fn publish_resource_discovery(
    client: &Client,
    cfg: &Config,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
) -> Result<String> {
    let discovery = collect_unbound_resource_discovery(cfg, hostname_override)?;
    let resource_id = derive_resource_lookup_id(&discovery);
    let url = format!(
        "{}/api/v1/resources/discovery",
        dserver_url.trim().trim_end_matches('/')
    );
    let response = client
        .post(&url)
        .header("X-Datum-Project", project_id.trim())
        .json(&discovery)
        .send()
        .with_context(|| format!("publish SmartSentinel role-free discovery {url}"))?;
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
            "dserver resource discovery returned resource_id '{}' but local discovery derived '{}'",
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
        // The instance-scoped resource id is intentionally ephemeral. For the local process
        // singleton only, hostname is a valid host-local mutex key; it is never used as identity.
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

fn write_report(
    path: &Path,
    report: &crate::agent::operational_reconciliation::OperationalReconciliationReport,
) -> Result<()> {
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
}
