use anyhow::Result;
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::process::Command;
use std::sync::Mutex;

use crate::core::ids;
use crate::core::model::{Edge, Entity, Snapshot};
use crate::core::plugin::Plugin;

use once_cell::sync::Lazy;

/// Cache de CPU acumulado por PID (segundos) para calcular delta entre ciclos.
/// best-effort.
static CPU_PREV_S: Lazy<Mutex<HashMap<u32, f64>>> = Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
struct PsProcRow {
    #[serde(rename = "ProcessName")]
    process_name: String,
    #[serde(rename = "Id")]
    id: u32,
    /// CPU acumulado em segundos (pode ser null em alguns processos)
    #[serde(rename = "CPU")]
    cpu_s: Option<f64>,
    /// WorkingSet em bytes
    #[serde(rename = "WorkingSet64")]
    working_set_bytes: Option<u64>,
}

#[derive(Debug, Default, Clone)]
struct ProcAgg {
    image_name: String,
    total_mem_kb: u64,
    instance_count: u32,
    sample_pids: Vec<u32>,
    cpu_pct_total: f64,
    cpu_s_delta_total: f64,
}

pub struct ProcessWindowsTasklistPlugin;

impl ProcessWindowsTasklistPlugin {
    pub fn new() -> Self {
        Self
    }
}

const SAMPLE_PIDS_MAX: usize = 5;

impl Plugin for ProcessWindowsTasklistPlugin {
    fn name(&self) -> &'static str {
        "process_windows_tasklist"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(windows) {
            return Ok(());
        }

        let proc_cfg = cfg
            .collectors
            .as_ref()
            .and_then(|c| c.process_windows.as_ref())
            .cloned()
            .unwrap_or_default();

        if !proc_cfg.enabled {
            return Ok(());
        }

        let names = proc_cfg.names.clone().unwrap_or_default();
        let allowset: HashSet<String> =
            names.into_iter().map(|s| s.trim().to_lowercase()).collect();

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

        // Intervalo de amostragem para CPU % (usa cycle_s como aproximação)
        let interval_s = cfg.agent.cycle_s.max(1) as f64;
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f64;

        // PowerShell: lista processos com CPU acumulado e WorkingSet
        // ProcessName NÃO tem .exe. Normalizamos com ".exe" para allowlist típica.
        let ps = r#"Get-Process | Select-Object ProcessName, Id, CPU, WorkingSet64 | ConvertTo-Json -Compress"#;

        let out = Command::new("powershell")
            .args(["-NoProfile", "-Command", ps])
            .output();

        let stdout = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
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

        // 1 processo => objeto; N => array
        let rows: Vec<PsProcRow> = match serde_json::from_str(&stdout) {
            Ok(v @ serde_json::Value::Array(_)) => serde_json::from_value(v).unwrap_or_default(),
            Ok(v) => serde_json::from_value(serde_json::Value::Array(vec![v])).unwrap_or_default(),
            Err(e) => {
                eprintln!("plugin={} status=warn err=parse_json {}", self.name(), e);
                return Ok(());
            }
        };

        let observed_at = snapshot.meta.observed_at_utc;
        let host_id = snapshot.meta.host_id.clone();

        let mut agg: HashMap<String, ProcAgg> = HashMap::new();
        let mut seen_pids: HashSet<u32> = HashSet::new();

        let mut cpu_prev = CPU_PREV_S.lock().unwrap();

        for r in rows {
            let image_name = format!("{}.exe", r.process_name);
            let image_lc = image_name.to_lowercase();
            let pid = r.id;
            seen_pids.insert(pid);

            let keep = match mode.as_str() {
                "all" => true,
                "top_mem" | "group_by_name" => allowset.is_empty() || allowset.contains(&image_lc),
                "allowlist" => allowset.contains(&image_lc),
                _ => allowset.is_empty() || allowset.contains(&image_lc),
            };
            if !keep {
                continue;
            }

            let mem_kb = r.working_set_bytes.unwrap_or(0) / 1024;

            let cur_cpu = r.cpu_s.unwrap_or(0.0);
            let prev_cpu = cpu_prev.get(&pid).copied().unwrap_or(cur_cpu);
            cpu_prev.insert(pid, cur_cpu);

            let delta_cpu_s = (cur_cpu - prev_cpu).max(0.0);
            let cpu_pct = (delta_cpu_s / (interval_s * cpu_cores)) * 100.0;

            let entry = agg.entry(image_lc.clone()).or_insert_with(|| ProcAgg {
                image_name: image_name.clone(),
                total_mem_kb: 0,
                instance_count: 0,
                sample_pids: Vec::new(),
                cpu_pct_total: 0.0,
                cpu_s_delta_total: 0.0,
            });

            entry.total_mem_kb = entry.total_mem_kb.saturating_add(mem_kb);
            entry.instance_count = entry.instance_count.saturating_add(1);
            entry.cpu_pct_total += cpu_pct;
            entry.cpu_s_delta_total += delta_cpu_s;

            if entry.sample_pids.len() < SAMPLE_PIDS_MAX {
                entry.sample_pids.push(pid);
            }
        }

        // best-effort cache cleanup
        cpu_prev.retain(|pid, _| seen_pids.contains(pid));
        drop(cpu_prev);

        let mut groups: Vec<(u64, String, ProcAgg)> = agg
            .into_iter()
            .map(|(k, v)| (v.total_mem_kb, k, v))
            .collect();

        if mode == "top_mem" {
            groups.sort_by(|a, b| b.0.cmp(&a.0));
        } else {
            groups.sort_by(|a, b| a.1.cmp(&b.1));
        }

        for (_mem_total, key, g) in groups.into_iter().take(max_entries) {
            let key_norm = ids::sanitize_id_component(&key);
            let proc_id = ids::process_group_id("windows", &key_norm);

            let ent = Entity {
                id: proc_id.clone(),
                kind: "process".to_string(),
                attrs: json!({
                    "os": "windows",
                    "image_name": g.image_name,
                    "instance_count": g.instance_count,
                    "cpu_pct_total": (g.cpu_pct_total * 100.0).round() / 100.0,
                    "cpu_s_delta_total": (g.cpu_s_delta_total * 1000.0).round() / 1000.0,
                    "mem_used_kb_total": g.total_mem_kb,
                    "mem_source": "working_set",
                    "sample_pids": g.sample_pids,
                    "aggregation": "by_image_name",
                    "interval_s": interval_s,
                    "cpu_cores": cpu_cores
                }),
                observed_at_utc: observed_at,
                ttl_s: 60,
            };

            let edge = Edge {
                from: host_id.clone(),
                to: proc_id,
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
