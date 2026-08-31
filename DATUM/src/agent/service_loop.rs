use std::cmp::min;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::config::{self, FileJsonConfig};
use crate::core::cycle::run_cycle;
use crate::core::model::Snapshot;
use crate::outputs::file_json::FileJsonOutput;
use crate::outputs::http_json::HttpJsonOutput;

const STOP_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug, Clone)]
pub struct ServiceLoopOptions {
    pub config_path: String,
    pub out_override: Option<String>,
    pub cycle_s_override: Option<u64>,
    pub node_id_override: Option<String>,
    pub hostname_override: Option<String>,
    pub stop_file: PathBuf,
    pub max_cycles: Option<u64>,
}

impl Default for ServiceLoopOptions {
    fn default() -> Self {
        Self {
            config_path: "config.toml".to_string(),
            out_override: None,
            cycle_s_override: None,
            node_id_override: None,
            hostname_override: None,
            stop_file: PathBuf::from(".smartsentinel-stop"),
            max_cycles: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownReason {
    StopFile,
    MaxCycles,
}

pub fn run_service_loop(options: ServiceLoopOptions) -> Result<()> {
    if options.stop_file.exists() {
        bail!(
            "SmartSentinel stop file already exists: {}. Remove it before starting the agent.",
            options.stop_file.display()
        );
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
        .unwrap_or_else(|| cfg.agent.node_id.clone());
    let cycle_s = options.cycle_s_override.unwrap_or(cfg.agent.cycle_s);
    if cycle_s == 0 {
        bail!("SmartSentinel cycle interval must be greater than zero seconds");
    }

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

    let http_output = match cfg.outputs.http.clone() {
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
    };

    let started_at_utc = Utc::now();
    let started_at = Instant::now();
    let mut sequence = 0u64;

    println!(
        "mode=smartsentinel event=start node_id={} cycle_s={} stop_file={} file_json={} http_json={} mutations=disabled",
        node_id,
        cycle_s,
        options.stop_file.display(),
        if file_output.is_some() { "enabled" } else { "disabled" },
        if http_output.is_some() { "enabled" } else { "disabled" }
    );

    let shutdown_reason = loop {
        if stop_requested(&options.stop_file) {
            break ShutdownReason::StopFile;
        }

        sequence = sequence.saturating_add(1);
        let cycle_started = Instant::now();

        match run_cycle(
            &node_id,
            hostname.as_deref(),
            &cfg.agent.schema_version,
            &cfg,
        ) {
            Ok(mut snapshot) => {
                annotate_heartbeat(
                    &mut snapshot,
                    sequence,
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
                    "mode=smartsentinel event=heartbeat seq={} status=ok cycle={} dur_ms={} uptime_ms={} entities={} edges={} events={} actions={} out={} side_effects=false",
                    sequence,
                    snapshot.meta.cycle_id,
                    cycle_started.elapsed().as_millis(),
                    started_at.elapsed().as_millis(),
                    snapshot.entities.len(),
                    snapshot.edges.len(),
                    snapshot.events.len(),
                    snapshot.actions.len(),
                    out_status.join(",")
                );
            }
            Err(e) => {
                println!(
                    "mode=smartsentinel event=heartbeat seq={} status=collection_error dur_ms={} uptime_ms={} err={} side_effects=false",
                    sequence,
                    cycle_started.elapsed().as_millis(),
                    started_at.elapsed().as_millis(),
                    e
                );
            }
        }

        if options.max_cycles.map(|n| sequence >= n).unwrap_or(false) {
            break ShutdownReason::MaxCycles;
        }

        if sleep_until_next_cycle(&options.stop_file, Duration::from_secs(cycle_s)) {
            break ShutdownReason::StopFile;
        }
    };

    if shutdown_reason == ShutdownReason::StopFile {
        acknowledge_stop_file(&options.stop_file)?;
    }

    println!(
        "mode=smartsentinel event=shutdown reason={} completed_cycles={} uptime_ms={} side_effects=false",
        match shutdown_reason {
            ShutdownReason::StopFile => "stop_file",
            ShutdownReason::MaxCycles => "max_cycles",
        },
        sequence,
        started_at.elapsed().as_millis()
    );

    Ok(())
}

fn annotate_heartbeat(
    snapshot: &mut Snapshot,
    sequence: u64,
    started_at_utc: DateTime<Utc>,
    uptime: Duration,
) {
    let Some(host) = snapshot
        .entities
        .iter_mut()
        .find(|entity| entity.id == snapshot.meta.host_id)
    else {
        return;
    };

    if !host.attrs.is_object() {
        host.attrs = json!({});
    }

    if let Some(attrs) = host.attrs.as_object_mut() {
        attrs.insert(
            "smartsentinel".to_string(),
            json!({
                "mode": "continuous",
                "heartbeat_sequence": sequence,
                "service_started_at_utc": started_at_utc,
                "service_uptime_ms": uptime.as_millis(),
                "safe_to_auto_execute": false,
                "side_effects_performed": false
            }),
        );
    }
}

fn stop_requested(stop_file: &Path) -> bool {
    stop_file.exists()
}

fn sleep_until_next_cycle(stop_file: &Path, duration: Duration) -> bool {
    let started = Instant::now();

    while started.elapsed() < duration {
        if stop_requested(stop_file) {
            return true;
        }

        let remaining = duration.saturating_sub(started.elapsed());
        thread::sleep(min(STOP_POLL_INTERVAL, remaining));
    }

    stop_requested(stop_file)
}

fn acknowledge_stop_file(stop_file: &Path) -> Result<()> {
    if !stop_file.exists() {
        return Ok(());
    }

    fs::remove_file(stop_file)
        .with_context(|| format!("remove SmartSentinel stop file {}", stop_file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Entity, Meta};
    use uuid::Uuid;

    #[test]
    fn heartbeat_annotation_is_explicitly_non_mutating() {
        let now = Utc::now();
        let host_id = "host:test".to_string();
        let mut snapshot = Snapshot {
            meta: Meta {
                schema_version: "test".to_string(),
                agent_version: "test".to_string(),
                cycle_id: Uuid::new_v4().to_string(),
                observed_at_utc: now,
                node_id: "local-01".to_string(),
                host_id: host_id.clone(),
            },
            entities: vec![Entity {
                id: host_id,
                kind: "host".to_string(),
                attrs: json!({"hostname": "test"}),
                observed_at_utc: now,
                ttl_s: 60,
            }],
            edges: vec![],
            events: vec![],
            actions: vec![],
        };

        annotate_heartbeat(&mut snapshot, 7, now, Duration::from_secs(3));

        let heartbeat = &snapshot.entities[0].attrs["smartsentinel"];
        assert_eq!(heartbeat["mode"], "continuous");
        assert_eq!(heartbeat["heartbeat_sequence"], 7);
        assert_eq!(heartbeat["safe_to_auto_execute"], false);
        assert_eq!(heartbeat["side_effects_performed"], false);
    }

    #[test]
    fn default_stop_file_is_stable() {
        assert_eq!(
            ServiceLoopOptions::default().stop_file,
            PathBuf::from(".smartsentinel-stop")
        );
    }
}
