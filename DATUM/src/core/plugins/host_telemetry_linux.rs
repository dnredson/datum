use anyhow::Result;
use serde_json::{json, Value as JsonValue};
use std::{fs, process::Command};

use crate::core::model::Snapshot;
use crate::core::plugin::Plugin;

use once_cell::sync::Lazy;
use std::sync::Mutex;

/// Cache para cálculo de CPU% global via delta de /proc/stat
static CPU_PREV: Lazy<Mutex<Option<(u64, u64)>>> = Lazy::new(|| Mutex::new(None));

pub struct HostTelemetryLinuxPlugin;

impl HostTelemetryLinuxPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for HostTelemetryLinuxPlugin {
    fn name(&self) -> &'static str {
        "host_telemetry_linux"
    }

    fn apply(&self, snapshot: &mut Snapshot, _cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(unix) {
            return Ok(());
        }

        let host_id = snapshot.meta.host_id.clone();
        let host_ent = match snapshot.entities.iter_mut().find(|e| e.id == host_id) {
            Some(e) => e,
            None => return Ok(()),
        };

        // OS info (best effort)
        let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
        let (os_name, os_version) = parse_os_release(&os_release);

        let arch = std::env::consts::ARCH.to_string();

        // Mem info
        let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
        let (mem_total_kb, mem_available_kb, swap_total_kb, swap_free_kb) =
            parse_meminfo_kb(&meminfo);
        let mem_used_kb = mem_total_kb.saturating_sub(mem_available_kb);
        let swap_used_kb = swap_total_kb.saturating_sub(swap_free_kb);

        // CPU logical
        let cpu_logical = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);

        // CPU usage %
        let stat = fs::read_to_string("/proc/stat").unwrap_or_default();
        let (total, idle) = parse_proc_stat_cpu(&stat).unwrap_or((0, 0));
        let cpu_usage_pct = calc_cpu_usage_pct(total, idle);

        // Disk usage (root)
        let (disk_total_kb, disk_used_kb, disk_free_kb) = disk_usage_root_kb();
        let uptime_seconds = parse_uptime_seconds(&fs::read_to_string("/proc/uptime").unwrap_or_default());
        let process_count = process_count();
        let kernel = kernel_version();
        let (load_1, load_5, load_15) = parse_load_average(
            &fs::read_to_string("/proc/loadavg").unwrap_or_default(),
        );

        let mut system_json: JsonValue = json!({
            "os_name": os_name,
            "os_version": os_version,
            "kernel": kernel,
            "arch": arch,
            "cpu_logical": cpu_logical,
            "cpu_usage_pct": cpu_usage_pct,
            "load_1": load_1,
            "load_5": load_5,
            "load_15": load_15,
            "uptime_seconds": uptime_seconds,
            "mem_total_mb": (mem_total_kb as f64 / 1024.0).round(),
            "mem_used_mb": (mem_used_kb as f64 / 1024.0).round(),
            "mem_available_mb": (mem_available_kb as f64 / 1024.0).round(),
            "mem_total_bytes": mem_total_kb.saturating_mul(1024),
            "mem_used_bytes": mem_used_kb.saturating_mul(1024),
            "mem_available_bytes": mem_available_kb.saturating_mul(1024),
            "swap_total_bytes": swap_total_kb.saturating_mul(1024),
            "swap_used_bytes": swap_used_kb.saturating_mul(1024),
            "process_count": process_count,
            "interfaces": network_interfaces(),
            "disk_root_total_gb": (disk_total_kb as f64 / (1024.0*1024.0)).round(),
            "disk_root_used_gb": (disk_used_kb as f64 / (1024.0*1024.0)).round(),
            "disk_root_free_gb": (disk_free_kb as f64 / (1024.0*1024.0)).round(),
            "filesystems": [{
                "mount": "/",
                "total_bytes": disk_total_kb.saturating_mul(1024),
                "used_bytes": disk_used_kb.saturating_mul(1024),
                "available_bytes": disk_free_kb.saturating_mul(1024)
            }],
        });

        if let Some(previous_system) = host_ent.attrs.get("system").and_then(JsonValue::as_object) {
            for key in [
                "container_runtime",
                "container_runtime_available",
                "container_count",
                "container_runtime_error",
            ] {
                if let Some(value) = previous_system.get(key) {
                    system_json[key] = value.clone();
                }
            }
        }

        if !host_ent.attrs.is_object() {
            host_ent.attrs = json!({});
        }
        let obj = host_ent.attrs.as_object_mut().unwrap();
        obj.insert("system".to_string(), system_json);

        Ok(())
    }
}

fn network_interfaces() -> Vec<JsonValue> {
    let counters = fs::read_to_string("/proc/net/dev")
        .ok()
        .map(|raw| {
            raw.lines()
                .filter_map(|line| {
                    let (name, values) = line.split_once(':')?;
                    let values = values.split_whitespace().collect::<Vec<_>>();
                    if values.len() < 11 {
                        return None;
                    }
                    Some((
                        name.trim().to_string(),
                        (
                            values[0].parse::<u64>().ok(),
                            values[8].parse::<u64>().ok(),
                            values[2].parse::<u64>().ok(),
                            values[10].parse::<u64>().ok(),
                        ),
                    ))
                })
                .collect::<std::collections::HashMap<_, _>>()
        })
        .unwrap_or_default();

    fs::read_dir("/sys/class/net")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let up = fs::read_to_string(entry.path().join("operstate"))
                .ok()
                .map(|state| state.trim().eq_ignore_ascii_case("up"));
            let addresses = interface_addresses(&name);
            let (rx_bytes, tx_bytes, rx_errors, tx_errors) =
                counters.get(&name).copied().unwrap_or((None, None, None, None));
            Some(json!({
                "name": name,
                "up": up,
                "addresses": addresses,
                "rx_bytes": rx_bytes,
                "tx_bytes": tx_bytes,
                "rx_errors": rx_errors,
                "tx_errors": tx_errors,
            }))
        })
        .collect()
}

fn interface_addresses(name: &str) -> Vec<String> {
    let output = match Command::new("ip")
        .args(["-j", "address", "show", "dev", name])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let value: JsonValue = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        _ => return Vec::new(),
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|interface| {
            interface
                .get("addr_info")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|address| address.get("local").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .collect()
}

fn parse_os_release(s: &str) -> (String, String) {
    let mut name = String::new();
    let mut version = String::new();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("NAME=") {
            name = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("VERSION_ID=") {
            version = v.trim_matches('"').to_string();
        }
    }
    if name.is_empty() {
        name = "linux".to_string();
    }
    (name, version)
}

fn parse_meminfo_kb(s: &str) -> (u64, u64, u64, u64) {
    let mut total = 0u64;
    let mut avail = 0u64;
    let mut swap_total = 0u64;
    let mut swap_free = 0u64;
    for line in s.lines() {
        if line.starts_with("MemTotal:") {
            total = extract_first_u64(line).unwrap_or(0);
        } else if line.starts_with("MemAvailable:") {
            avail = extract_first_u64(line).unwrap_or(0);
        } else if line.starts_with("SwapTotal:") {
            swap_total = extract_first_u64(line).unwrap_or(0);
        } else if line.starts_with("SwapFree:") {
            swap_free = extract_first_u64(line).unwrap_or(0);
        }
    }
    (total, avail, swap_total, swap_free)
}

fn extract_first_u64(s: &str) -> Option<u64> {
    for part in s.split_whitespace() {
        if let Ok(v) = part.parse::<u64>() {
            return Some(v);
        }
    }
    None
}

fn parse_uptime_seconds(s: &str) -> Option<u64> {
    s.split_whitespace()
        .next()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.round() as u64)
}

fn parse_load_average(s: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut values = s
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split(',')
        .filter_map(|value| value.parse::<f64>().ok());
    (values.next(), values.next(), values.next())
}

fn kernel_version() -> Option<String> {
    Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
}

fn process_count() -> Option<u64> {
    fs::read_dir("/proc").ok().map(|entries| {
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| name.parse::<u64>().ok())
            .count() as u64
    })
}

fn parse_proc_stat_cpu(s: &str) -> Option<(u64, u64)> {
    // cpu  user nice system idle iowait irq softirq steal guest guest_nice
    let line = s.lines().next()?;
    if !line.starts_with("cpu ") {
        return None;
    }
    let nums: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse::<u64>().ok())
        .collect();
    if nums.len() < 4 {
        return None;
    }
    let idle = nums[3] + nums.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = nums.iter().sum();
    Some((total, idle))
}

fn calc_cpu_usage_pct(total: u64, idle: u64) -> f64 {
    let mut prev = CPU_PREV.lock().unwrap();
    let pct = if let Some((pt, pi)) = *prev {
        let dt = total.saturating_sub(pt);
        let di = idle.saturating_sub(pi);
        if dt == 0 {
            0.0
        } else {
            ((dt - di) as f64 / dt as f64) * 100.0
        }
    } else {
        0.0
    };
    *prev = Some((total, idle));
    (pct * 100.0).round() / 100.0
}

fn disk_usage_root_kb() -> (u64, u64, u64) {
    // Best-effort via `df -kP /`
    let out = std::process::Command::new("df").args(["-kP", "/"]).output();

    if let Ok(o) = out {
        let s = String::from_utf8_lossy(&o.stdout);
        let mut lines = s.lines();
        lines.next();
        if let Some(line) = lines.next() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 4 {
                let total = cols[1].parse::<u64>().unwrap_or(0);
                let used = cols[2].parse::<u64>().unwrap_or(0);
                let avail = cols[3].parse::<u64>().unwrap_or(0);
                return (total, used, avail);
            }
        }
    }
    (0, 0, 0)
}
