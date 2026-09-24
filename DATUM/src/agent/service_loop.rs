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

use crate::agent::dmonitor_projection::{
    project_telemetry_to_dmonitor_observations, DMonitorProjectionContext,
};
use crate::config::{self, FileJsonConfig};
use crate::core::cycle::{run_cycle_for_node_identity, run_observation_only_cycle_for_node_identity};
use crate::telemetry::{attach_host_telemetry_with_peers, build_host_telemetry, write_telemetry_atomic};
use crate::core::model::Snapshot;
use crate::outputs::dmonitor_http::DMonitorHttpOutput;
use crate::outputs::file_json::FileJsonOutput;
use crate::outputs::http_json::HttpJsonOutput;
use crate::outputs::telemetry_http::TelemetryHttpOutput;

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
    pub telemetry_out_override: Option<PathBuf>,
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
            telemetry_out_override: None,
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

    let (http_publish_url, http_project_id, http_timeout_s) = if options
        .http_url_override
        .is_some()
        || options.http_project_id_override.is_some()
    {
        let url = options
            .http_url_override
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("SmartSentinel HTTP publishing requires a non-empty URL"))?;
        let project_id = options
            .http_project_id_override
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("SmartSentinel HTTP publishing requires a non-empty project id"))?;
        (
            Some(url),
            Some(project_id),
            options.http_timeout_s_override.unwrap_or(10),
        )
    } else {
        match cfg.outputs.http.clone() {
            Some(http_cfg) if http_cfg.enabled => (
                Some(http_cfg.url),
                Some(http_cfg
                    .project_id
                    .unwrap_or_else(|| "default-project".to_string())),
                http_cfg.timeout_s.unwrap_or(10),
            ),
            _ => (None, None, 10),
        }
    };

    let http_output = match (http_publish_url.clone(), http_project_id.clone()) {
        (Some(url), Some(project_id)) => Some(HttpJsonOutput::new(url, project_id, http_timeout_s)?),
        _ => None,
    };
    let telemetry_http_output = match (
        http_output.as_ref(),
        http_publish_url.as_deref(),
        http_project_id.as_deref(),
    ) {
        (Some(_), Some(snapshot_url), Some(project_id)) => Some(TelemetryHttpOutput::new(
            telemetry_publish_url(snapshot_url, &node_id),
            project_id.to_string(),
            http_timeout_s,
        )?),
        _ => None,
    };

    // ---- DMonitor canonical projection/publication (Phase 114B2) ----
    // Piggybacks on the SAME HTTP publishing configuration as the legacy
    // outputs above (no separate DMonitor CLI/config surface) -- an
    // ADDITIONAL output path, not a replacement. `dmonitor_context` is a
    // single process-epoch identity created ONCE here, at service-loop
    // startup, and reused for every collection for the lifetime of this
    // process (section 20's explicit prohibition on creating a new
    // `monitor_instance_id` per cycle); its sequence counter is therefore
    // monotonic across the whole run.
    let dmonitor_output = match (
        http_output.as_ref(),
        http_publish_url.as_deref(),
        http_project_id.as_deref(),
    ) {
        (Some(_), Some(snapshot_url), Some(project_id)) => Some(DMonitorHttpOutput::new(
            dserver_base_url(snapshot_url),
            project_id.to_string(),
            http_timeout_s,
        )?),
        _ => None,
    };
    let mut dmonitor_context = DMonitorProjectionContext::new_process_epoch(&node_id);

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
        "mode=smartsentinel event=start node_id={} cycle_s={} heartbeat_s={} stop_file={} lock_file={} file_json={} http_json={} telemetry_http_json={} dmonitor_json={} dmonitor_monitor_instance_id={} mutations=disabled",
        node_id,
        cycle_s,
        options.heartbeat_s,
        options.stop_file.display(),
        instance_lock_path.display(),
        if file_output.is_some() { "enabled" } else { "disabled" },
        if http_output.is_some() { "enabled" } else { "disabled" },
        if telemetry_http_output.is_some() { "enabled" } else { "disabled" },
        if dmonitor_output.is_some() { "enabled" } else { "disabled" },
        dmonitor_context.monitor_instance_id()
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

                let telemetry_project_id = options
                    .http_project_id_override
                    .as_deref()
                    .or_else(|| {
                        cfg.outputs
                            .http
                            .as_ref()
                            .and_then(|http| http.project_id.as_deref())
                    });
                let control_plane_url = options
                    .http_url_override
                    .as_deref()
                    .or_else(|| cfg.outputs.http.as_ref().map(|http| http.url.as_str()));
                let peer_targets = cfg
                    .collectors
                    .as_ref()
                    .and_then(|collectors| collectors.connection_ping.as_ref())
                    .map(|ping| ping.peer_targets.as_slice())
                    .unwrap_or(&[]);
                let telemetry = match attach_host_telemetry_with_peers(
                    &mut snapshot,
                    telemetry_project_id,
                    collection_sequence,
                    control_plane_url,
                    peer_targets,
                ) {
                    Ok(telemetry) => telemetry,
                    Err(error) => {
                        println!(
                            "mode=smartsentinel event=collection seq={} status=telemetry_error dur_ms={} uptime_ms={} heartbeat_seq={} err={} side_effects=false",
                            collection_sequence,
                            cycle_started.elapsed().as_millis(),
                            started_at.elapsed().as_millis(),
                            heartbeat_sequence.load(Ordering::Relaxed),
                            error
                        );
                        continue;
                    }
                };

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

                match &telemetry_http_output {
                    Some(out) => match out.emit(&telemetry) {
                        Ok(_) => out_status.push("telemetry_http=ok".to_string()),
                        Err(e) => out_status.push(format!("telemetry_http=err({})", e)),
                    },
                    None => out_status.push("telemetry_http=disabled".to_string()),
                }

                // Separate, structurally read-only DMonitor-source
                // collection (section 9) -- never reuses `snapshot`/
                // `telemetry` above, which came from `run_cycle_for_node_identity`
                // (unconditionally including the planning/recovery/execution
                // plugins, even though this loop's own `cfg` mutation at
                // startup disables them internally). `dmonitor_context` is
                // NOT reset here: it is the same context created once at
                // service-loop startup, so its sequence keeps advancing
                // across every cycle (section 20).
                match run_observation_only_cycle_for_node_identity(
                    &node_id,
                    hostname.as_deref(),
                    &cfg.agent.schema_version,
                    &cfg,
                )
                .map_err(|error| error.to_string())
                .and_then(|observation_snapshot| build_host_telemetry(&observation_snapshot, None, 1))
                {
                    Ok(observation_telemetry) => {
                        match project_telemetry_to_dmonitor_observations(
                            &observation_telemetry,
                            &mut dmonitor_context,
                        ) {
                            Ok(observations) => match &dmonitor_output {
                                Some(out) => {
                                    let payloads: Vec<(String, serde_json::Value)> = observations
                                        .iter()
                                        .filter_map(|observation| {
                                            serde_json::to_value(observation)
                                                .ok()
                                                .map(|value| (observation.observation_id.clone(), value))
                                        })
                                        .collect();
                                    let report = out.publish_all(&payloads);
                                    out_status.push(format!(
                                        "dmonitor=ok(accepted={},idempotent_replay={},failed={},not_attempted={})",
                                        report.accepted.len(),
                                        report.idempotent_replay.len(),
                                        report.failed.is_some(),
                                        report.not_attempted.len()
                                    ));
                                }
                                None => out_status.push("dmonitor=disabled".to_string()),
                            },
                            Err(error) => out_status.push(format!("dmonitor=err(projection:{error})")),
                        }
                    }
                    Err(error) => out_status.push(format!("dmonitor=err(collection:{error})")),
                }

                if let Some(path) = &options.telemetry_out_override {
                    match write_telemetry_atomic(path, &telemetry) {
                        Ok(_) => out_status.push("telemetry_json=ok".to_string()),
                        Err(error) => out_status.push(format!("telemetry_json=err({})", error)),
                    }
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

fn dserver_base_url(snapshot_url: &str) -> String {
    let snapshot_url = snapshot_url.trim_end_matches('/');
    snapshot_url
        .strip_suffix("/api/v1/snapshots")
        .unwrap_or(snapshot_url)
        .trim_end_matches('/')
        .to_string()
}

fn telemetry_publish_url(snapshot_url: &str, node_id: &str) -> String {
    format!("{}/api/v1/sentinels/{}/telemetry", dserver_base_url(snapshot_url), node_id)
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
