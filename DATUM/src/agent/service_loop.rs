use std::cmp::min;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde_json::json;

use crate::config::{self, FileJsonConfig};
use crate::core::cycle::run_cycle_for_node_identity;
use crate::core::model::Snapshot;
use crate::outputs::file_json::FileJsonOutput;
use crate::outputs::http_json::HttpJsonOutput;

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);
const SHUTDOWN_NONE: u8 = 0;
const SHUTDOWN_CTRL_C: u8 = 1;
const SHUTDOWN_STOP_FILE: u8 = 2;
const SHUTDOWN_MAX_CYCLES: u8 = 3;

#[derive(Debug, Clone)]
pub struct ServiceLoopOptions {
    pub config_path: String,
    pub out_override: Option<String>,
    pub cycle_s_override: Option<u64>,
    pub heartbeat_s: u64,
    pub node_id_override: Option<String>,
    pub hostname_override: Option<String>,
    pub stop_file: PathBuf,
    pub max_cycles: Option<u64>,
    /// Optional command-line override for the read-only HTTP observation sink.
    pub http_url_override: Option<String>,
    pub http_project_id_override: Option<String>,
    pub http_timeout_s_override: Option<u64>,
}

impl Default for ServiceLoopOptions {
    fn default() -> Self {
        Self {
            config_path: "config.toml".to_string(),
            out_override: None,
            cycle_s_override: None,
            heartbeat_s: 1,
            node_id_override: None,
            hostname_override: None,
            stop_file: PathBuf::from(".smartsentinel-stop"),
            max_cycles: None,
            http_url_override: None,
            http_project_id_override: None,
            http_timeout_s_override: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownReason {
    CtrlC,
    StopFile,
    MaxCycles,
}

impl ShutdownReason {
    fn from_signal(value: u8) -> Option<Self> {
        match value {
            SHUTDOWN_CTRL_C => Some(Self::CtrlC),
            SHUTDOWN_STOP_FILE => Some(Self::StopFile),
            SHUTDOWN_MAX_CYCLES => Some(Self::MaxCycles),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::CtrlC => "ctrl_c",
            Self::StopFile => "stop_file",
            Self::MaxCycles => "max_cycles",
        }
    }
}

pub fn instance_lock_path(node_id: &str) -> PathBuf {
    let safe_node_id = crate::core::ids::sanitize_id_component(node_id);
    std::env::temp_dir().join(format!("smartsentinel-{}.lock", safe_node_id))
}

fn acquire_instance_lock(node_id: &str) -> Result<(File, PathBuf)> {
    let path = instance_lock_path(node_id);
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("open SmartSentinel instance lock {}", path.display()))?;

    if let Err(e) = file.try_lock_exclusive() {
        bail!(
            "SmartSentinel instance already running for node_id={} (lock={}): {}",
            node_id,
            path.display(),
            e
        );
    }

    file.set_len(0)
        .with_context(|| format!("truncate SmartSentinel instance lock {}", path.display()))?;
    write!(file, "pid={}\nnode_id={}\n", std::process::id(), node_id)
        .with_context(|| format!("write SmartSentinel instance lock {}", path.display()))?;
    file.flush()
        .with_context(|| format!("flush SmartSentinel instance lock {}", path.display()))?;

    Ok((file, path))
}

pub fn run_service_loop(options: ServiceLoopOptions) -> Result<()> {
    if options.stop_file.exists() {
        bail!(
            "SmartSentinel stop file already exists: {}. Remove it before starting the agent.",
            options.stop_file.display()
        );
    }

    if options.max_cycles == Some(0) {
        bail!("SmartSentinel max-cycles must be greater than zero when provided");
    }
    if options.heartbeat_s == 0 {
        bail!("SmartSentinel heartbeat interval must be greater than zero seconds");
    }

    let mut cfg = config::load_config(&options.config_path)?;

    // O1 is observation-only. Preserve historical configuration on disk, but make
    // the continuous SmartSentinel loop incapable of executing remediation actions.
    if let Some(auto_recovery) = cfg.auto_recovery.as_mut() {
        auto_recovery.enabled = false;
        auto_recovery.dry_run = Some(true);
    }
    if let Some(desired_state) = cfg.desired_state.as_mut() {
        desired_state.enabled = false;
    }

    let node_id = options
        .node_id_override
        .clone()
        .unwrap_or_else(|| cfg.agent.node_id.clone());
    let cycle_s = options.cycle_s_override.unwrap_or(cfg.agent.cycle_s);
    if cycle_s == 0 {
        bail!("SmartSentinel cycle interval must be greater than zero seconds");
    }

    // Hold an OS-backed exclusive file lock for the entire process lifetime. The lock
    // lives in the host temp directory so a second clone/working directory cannot
    // launch another SmartSentinel for the same logical node. The OS releases it on
    // normal exit and abnormal process termination; the small lock file may remain.
    let (_instance_lock, instance_lock_path) = acquire_instance_lock(&node_id)?;

    let hostname = options.hostname_override.or(cfg.agent.hostname.clone());

    let file_cfg = cfg
        .outputs
        .file_json
        .clone()
        .unwrap_or(FileJsonConfig {
            enabled: true,
            path: "snapshot.json".to_string(),
        });
    let out_path = options.out_override.unwrap_or(file_cfg.path);
    let file_output = if file_cfg.enabled {
        Some(FileJsonOutput::new(out_path))
    } else {
        None
    };

    let http_output = if options.http_url_override.is_some()
        || options.http_project_id_override.is_some()
    {
        let url = options
            .http_url_override
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| bail!("SmartSentinel HTTP publishing requires a non-empty URL"))?;
        let project_id = options
            .http_project_id_override
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| bail!("SmartSentinel HTTP publishing requires a non-empty project id"))?;
        Some(HttpJsonOutput::new(
            url,
            project_id,
            options.http_timeout_s_override.unwrap_or(10),
        )?)
    } else {
        match cfg.outputs.http.clone() {
            Some(http_cfg) if http_cfg.enabled => {
                let project_id = http_cfg
                    .project_id
                    .clone()
                    .unwrap_or_else(|| "default-project".to_string());
                Some(HttpJsonOutput::new(
                    http_cfg.url,
                    project_id,
                    http_cfg.timeout_s.unwrap_or(10),
                )?)
            }
            _ => None,
        }
    };

    let started_at_utc = Utc::now();
    let started_at = Instant::now();
    let shutdown_signal = Arc::new(AtomicU8::new(SHUTDOWN_NONE));
    let heartbeat_sequence = Arc::new(AtomicU64::new(0));

    install_ctrlc_handler(Arc::clone(&shutdown_signal))?;

    let heartbeat_thread = spawn_liveness_heartbeat(
        node_id.clone(),
        options.stop_file.clone(),
        options.heartbeat_s,
        started_at,
        Arc::clone(&heartbeat_sequence),
        Arc::clone(&shutdown_signal),
    );

    println!(
        "mode=smartsentinel event=start node_id={} cycle_s={} heartbeat_s={} stop_file={} lock_file={} file_json={} http_json={} mutations=disabled",
        node_id,
        cycle_s,
        options.heartbeat_s,
        options.stop_file.display(),
        instance_lock_path.display(),
        if file_output.is_some() { "enabled" } else { "disabled" },
        if http_output.is_some() { "enabled" } else { "disabled" }
    );

    let mut collection_sequence = 0u64;

    loop {
        detect_stop_file(&options.stop_file, &shutdown_signal);
        if shutdown_requested(&shutdown_signal) {
            break;
        }

        collection_sequence = collection_sequence.saturating_add(1);
        let cycle_started = Instant::now();

        match run_cycle_for_node_identity(
            &node_id,
            hostname.as_deref(),
            &cfg.agent.schema_version,
            &cfg,
        ) {
            Ok(mut snapshot) => {
                annotate_heartbeat(
                    &mut snapshot,
                    heartbeat_sequence.load(Ordering::Relaxed),
                    started_at_utc,
                    started_at.elapsed(),
                );

                let mut out_status = Vec::new();

                match &file_output {
                    Some(out) => match out.emit(&snapshot) {
                        Ok(_) => out_status.push("file_json=ok".to_string()),
                        Err(e) => out_status.push(format!("file_json=err({})", e)),
                    },
                    None => out_status.push("file_json=disabled".to_string()),
                }

                match &http_output {
                    Some(out) => match out.emit(&snapshot) {
                        Ok(_) => out_status.push("http_json=ok".to_string()),
                        Err(e) => out_status.push(format!("http_json=err({})", e)),
                    },
                    None => out_status.push("http_json=disabled".to_string()),
                }

                println!(
                    "mode=smartsentinel event=collection seq={} status=ok cycle={} dur_ms={} uptime_ms={} heartbeat_seq={} entities={} edges={} events={} actions={} out={} side_effects=false",
                    collection_sequence,
                    snapshot.meta.cycle_id,
                    cycle_started.elapsed().as_millis(),
                    started_at.elapsed().as_millis(),
                    heartbeat_sequence.load(Ordering::Relaxed),
                    snapshot.entities.len(),
                    snapshot.edges.len(),
                    snapshot.events.len(),
                    snapshot.actions.len(),
                    out_status.join(",")
                );
            }
            Err(e) => {
                println!(
                    "mode=smartsentinel event=collection seq={} status=collection_error dur_ms={} uptime_ms={} heartbeat_seq={} err={} side_effects=false",
                    collection_sequence,
                    cycle_started.elapsed().as_millis(),
                    started_at.elapsed().as_millis(),
                    heartbeat_sequence.load(Ordering::Relaxed),
                    e
                );
            }
        }

        detect_stop_file(&options.stop_file, &shutdown_signal);

        if options
            .max_cycles
            .map(|n| collection_sequence >= n)
            .unwrap_or(false)
        {
            let _ = shutdown_signal.compare_exchange(
                SHUTDOWN_NONE,
                SHUTDOWN_MAX_CYCLES,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }

        if shutdown_requested(&shutdown_signal) {
            break;
        }

        wait_between_collections(
            &options.stop_file,
            &shutdown_signal,
            Duration::from_secs(cycle_s),
        );
    }

    let reason = ShutdownReason::from_signal(shutdown_signal.load(Ordering::SeqCst))
        .unwrap_or(ShutdownReason::CtrlC);

    if reason == ShutdownReason::StopFile {
        acknowledge_stop_file(&options.stop_file)?;
    }

    if heartbeat_thread.join().is_err() {
        eprintln!("mode=smartsentinel event=liveness status=thread_join_error");
    }

    println!(
        "mode=smartsentinel event=shutdown reason={} completed_collections={} heartbeat_seq={} uptime_ms={} side_effects=false",
        reason.as_str(),
        collection_sequence,
        heartbeat_sequence.load(Ordering::Relaxed),
        started_at.elapsed().as_millis()
    );

    Ok(())
}

fn install_ctrlc_handler(shutdown_signal: Arc<AtomicU8>) -> Result<()> {
    ctrlc::set_handler(move || {
        let _ = shutdown_signal.compare_exchange(
            SHUTDOWN_NONE,
            SHUTDOWN_CTRL_C,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    })
    .context("install SmartSentinel Ctrl+C handler")
}

fn spawn_liveness_heartbeat(
    node_id: String,
    stop_file: PathBuf,
    heartbeat_s: u64,
    started_at: Instant,
    heartbeat_sequence: Arc<AtomicU64>,
    shutdown_signal: Arc<AtomicU8>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let interval = Duration::from_secs(heartbeat_s);

        while !shutdown_requested(&shutdown_signal) {
            detect_stop_file(&stop_file, &shutdown_signal);
            if shutdown_requested(&shutdown_signal) {
                break;
            }

            let seq = heartbeat_sequence.fetch_add(1, Ordering::Relaxed) + 1;
            println!(
                "mode=smartsentinel event=liveness seq={} node_id={} status=alive uptime_ms={} side_effects=false",
                seq,
                node_id,
                started_at.elapsed().as_millis()
            );

            wait_with_shutdown(&stop_file, &shutdown_signal, interval);
        }
    })
}

pub fn annotate_heartbeat(
    snapshot: &mut Snapshot,
    sequence: u64,
    started_at_utc: DateTime<Utc>,
    uptime: Duration,
) {
    let host_id = snapshot.meta.host_id.clone();
    let Some(host) = snapshot
        .entities
        .iter_mut()
        .find(|entity| entity.id == host_id)
    else {
        return;
    };

    if !host.attrs.is_object() {
        host.attrs = json!({});
    }

    if let Some(attrs) = host.attrs.as_object_mut() {
        let uptime_ms = uptime.as_millis().min(u64::MAX as u128) as u64;
        attrs.insert(
            "smartsentinel".to_string(),
            json!({
                "mode": "continuous",
                "heartbeat_sequence": sequence,
                "service_started_at_utc": started_at_utc,
                "service_uptime_ms": uptime_ms,
                "safe_to_auto_execute": false,
                "side_effects_performed": false
            }),
        );
    }
}

fn shutdown_requested(shutdown_signal: &AtomicU8) -> bool {
    shutdown_signal.load(Ordering::SeqCst) != SHUTDOWN_NONE
}

fn detect_stop_file(stop_file: &Path, shutdown_signal: &AtomicU8) {
    if stop_file.exists() {
        let _ = shutdown_signal.compare_exchange(
            SHUTDOWN_NONE,
            SHUTDOWN_STOP_FILE,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
    }
}

fn wait_between_collections(
    stop_file: &Path,
    shutdown_signal: &AtomicU8,
    duration: Duration,
) {
    wait_with_shutdown(stop_file, shutdown_signal, duration);
}

fn wait_with_shutdown(stop_file: &Path, shutdown_signal: &AtomicU8, duration: Duration) {
    let started = Instant::now();

    while started.elapsed() < duration {
        detect_stop_file(stop_file, shutdown_signal);
        if shutdown_requested(shutdown_signal) {
            return;
        }

        let remaining = duration.saturating_sub(started.elapsed());
        thread::sleep(min(STOP_POLL_INTERVAL, remaining));
    }
}

fn acknowledge_stop_file(stop_file: &Path) -> Result<()> {
    if !stop_file.exists() {
        return Ok(());
    }

    fs::remove_file(stop_file)
        .with_context(|| format!("remove SmartSentinel stop file {}", stop_file.display()))
}
