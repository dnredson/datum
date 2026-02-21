use anyhow::Result;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::core::ids;
use crate::core::model::{Edge, Entity, Snapshot};
use crate::core::plugin::Plugin;

#[derive(Debug, Default, Clone)]
struct ProcAgg {
    name: String,
    total_rss_kb: u64,
    instance_count: u32,
    cpu_pct_total: f64,
    sample_pids: Vec<u32>,
}

pub struct ProcessLinuxPsPlugin;

impl ProcessLinuxPsPlugin {
    pub fn new() -> Self {
        Self
    }
}

const SAMPLE_PIDS_MAX: usize = 5;

impl Plugin for ProcessLinuxPsPlugin {
    fn name(&self) -> &'static str {
        "process_linux_ps"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(unix) {
            return Ok(());
        }

        let proc_cfg = cfg
            .collectors
            .as_ref()
            .and_then(|c| c.process_linux.as_ref())
            .cloned()
            .unwrap_or_default();

        if !proc_cfg.enabled {
            return Ok(());
        }

        let names = proc_cfg.names.clone().unwrap_or_default();
        let allowset: HashSet<String> = names
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .collect();

        // allowlist-first default: if names present and mode missing -> allowlist
        let mode = match proc_cfg.mode.as_deref() {
            Some(m) => m.to_string(),
            None => {
                if !allowset.is_empty() {
                    "allowlist".to_string()
                } else {
                    "top_mem".to_string()
                }
            }
        };

        let max_entries = proc_cfg.max_entries.unwrap_or(50);

        // ps: pid, comm, %cpu, rss(KB)
        let out = Command::new("ps")
            .args(["-eo", "pid=,comm=,pcpu=,rss="])
            .output();

        let stdout = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            Ok(o) => {
                eprintln!(
                    "plugin={} status=skipped err={}",
                    self.name(),
                    String::from_utf8_lossy(&o.stderr).trim()
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!("plugin={} status=skipped err={}", self.name(), e);
                return Ok(());
            }
        };

        let observed_at = snapshot.meta.observed_at_utc;
        let host_id = snapshot.meta.host_id.clone();

        let mut agg: HashMap<String, ProcAgg> = HashMap::new();

        for line in stdout.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 4 {
                continue;
            }
            let pid = cols[0].parse::<u32>().ok();
            let comm = cols[1].to_string();
            let cpu = cols[2].parse::<f64>().unwrap_or(0.0);
            let rss = cols[3].parse::<u64>().unwrap_or(0);

            let comm_lc = comm.to_lowercase();

            let keep = match mode.as_str() {
                "all" => true,
                "top_mem" | "group_by_name" => allowset.is_empty() || allowset.contains(&comm_lc),
                "allowlist" => allowset.contains(&comm_lc),
                _ => allowset.is_empty() || allowset.contains(&comm_lc),
            };
            if !keep {
                continue;
            }

            let entry = agg.entry(comm_lc.clone()).or_insert_with(|| ProcAgg {
                name: comm.clone(),
                total_rss_kb: 0,
                instance_count: 0,
                cpu_pct_total: 0.0,
                sample_pids: Vec::new(),
            });

            entry.total_rss_kb = entry.total_rss_kb.saturating_add(rss);
            entry.instance_count = entry.instance_count.saturating_add(1);
            entry.cpu_pct_total += cpu;

            if let Some(pid) = pid {
                if entry.sample_pids.len() < SAMPLE_PIDS_MAX {
                    entry.sample_pids.push(pid);
                }
            }
        }

        let mut groups: Vec<(u64, String, ProcAgg)> = agg
            .into_iter()
            .map(|(k, v)| (v.total_rss_kb, k, v))
            .collect();

        if mode == "top_mem" {
            groups.sort_by(|a, b| b.0.cmp(&a.0));
        } else {
            groups.sort_by(|a, b| a.1.cmp(&b.1));
        }

        for (_rss_total, key, g) in groups.into_iter().take(max_entries) {
            let key_norm = ids::sanitize_id_component(&key);
            let id = ids::process_group_id("linux", &key_norm);

            let ent = Entity {
                id: id.clone(),
                kind: "process".to_string(),
                attrs: json!({
                    "os": "linux",
                    "name": g.name,
                    "instance_count": g.instance_count,
                    "cpu_pct_total": (g.cpu_pct_total * 100.0).round() / 100.0,
                    "mem_used_kb_total": g.total_rss_kb,
                    "mem_source": "rss",
                    "sample_pids": g.sample_pids,
                    "aggregation": "by_comm",
                }),
                observed_at_utc: observed_at,
                ttl_s: 60,
            };

            let edge = Edge {
                from: host_id.clone(),
                to: id,
                rel: "runs".to_string(),
                attrs: json!({"runtime":"process"}),
                observed_at_utc: observed_at,
            };

            snapshot.entities.push(ent);
            snapshot.edges.push(edge);
        }

        Ok(())
    }
}
