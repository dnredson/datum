use anyhow::Result;
use serde_json::json;
use std::process::Command;

use crate::core::ids;
use crate::core::model::{Edge, Entity, Snapshot};
use crate::core::plugin::Plugin;

pub struct ConnectionPingWindowsPlugin;

impl ConnectionPingWindowsPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for ConnectionPingWindowsPlugin {
    fn name(&self) -> &'static str {
        "connection_ping_windows"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(windows) {
            return Ok(());
        }

        let ping_cfg = cfg
            .collectors
            .as_ref()
            .and_then(|c| c.connection_ping.as_ref())
            .cloned()
            .unwrap_or_default();

        if !ping_cfg.enabled {
            return Ok(());
        }

        let targets = ping_cfg
            .targets
            .unwrap_or_else(|| vec!["1.1.1.1".to_string()]);
        let count = ping_cfg.count.unwrap_or(4);
        let timeout_s = ping_cfg.timeout_s.unwrap_or(1);

        let observed_at = snapshot.meta.observed_at_utc;
        let host_id = snapshot.meta.host_id.clone();

        for t in targets {
            let res = ping_and_parse(&t, count, timeout_s);

            let t_norm = ids::sanitize_id_component(&t.to_lowercase());
            let conn_id = ids::connection_id(&host_id, &t_norm);

            let ent = Entity {
                id: conn_id.clone(),
                kind: "connection".to_string(),
                attrs: json!({
                    "from": host_id,
                    "to": t,
                    "method": "ping",
                    "samples": count,
                    "timeout_s": timeout_s,
                    "available": res.available,
                    "loss_pct": res.loss_pct,
                    "rtt_min_ms": res.rtt_min_ms,
                    "rtt_avg_ms": res.rtt_avg_ms,
                    "rtt_max_ms": res.rtt_max_ms,
                    "raw": res.raw,
                }),
                observed_at_utc: observed_at,
                ttl_s: 60,
            };

            let edge = Edge {
                from: snapshot.meta.host_id.clone(),
                to: conn_id,
                rel: "measures".to_string(),
                attrs: json!({"method":"ping"}),
                observed_at_utc: observed_at,
            };

            snapshot.entities.push(ent);
            snapshot.edges.push(edge);
        }

        Ok(())
    }
}

struct PingResult {
    available: bool,
    loss_pct: u64,
    rtt_min_ms: Option<u64>,
    rtt_avg_ms: Option<u64>,
    rtt_max_ms: Option<u64>,
    raw: String,
}

fn ping_and_parse(target: &str, count: u32, timeout_s: u32) -> PingResult {
    // Windows ping: -n <count> -w <timeout_ms>
    let timeout_ms = timeout_s.saturating_mul(1000);
    let out = Command::new("ping")
        .args([
            "-n",
            &count.to_string(),
            "-w",
            &timeout_ms.to_string(),
            target,
        ])
        .output();

    match out {
        Ok(o) => {
            let raw = format!(
                "{}\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            parse_windows_ping_output(&raw)
        }
        Err(e) => PingResult {
            available: false,
            loss_pct: 100,
            rtt_min_ms: None,
            rtt_avg_ms: None,
            rtt_max_ms: None,
            raw: format!("ping exec error: {}", e),
        },
    }
}

fn parse_windows_ping_output(raw: &str) -> PingResult {
    let lower = raw.to_lowercase();
    let available = lower.contains("ttl=");
    let mut loss_pct: u64 = if available { 0 } else { 100 };

    if let Some(pct) = extract_first_percentage(&lower) {
        loss_pct = pct;
    }
    let (min_ms, max_ms, avg_ms) = extract_rtt_triplet_ms(&lower);

    PingResult {
        available,
        loss_pct,
        rtt_min_ms: min_ms,
        rtt_avg_ms: avg_ms,
        rtt_max_ms: max_ms,
        raw: raw.trim().to_string(),
    }
}

fn extract_first_percentage(s: &str) -> Option<u64> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'%' {
            let mut j = i;
            while j > 0 && bytes[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j < i {
                if let Ok(v) = s[j..i].parse::<u64>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn extract_rtt_triplet_ms(s: &str) -> (Option<u64>, Option<u64>, Option<u64>) {
    let mut nums: Vec<u64> = Vec::new();
    let mut i = 0;
    let bytes = s.as_bytes();
    while i + 1 < bytes.len() {
        if bytes[i] == b'm' && bytes[i + 1] == b's' {
            let mut j = i;
            while j > 0 && bytes[j - 1].is_ascii_digit() {
                j -= 1;
            }
            if j < i {
                if let Ok(v) = s[j..i].parse::<u64>() {
                    nums.push(v);
                }
            }
        }
        i += 1;
    }
    if nums.len() >= 3 {
        (Some(nums[0]), Some(nums[1]), Some(nums[2]))
    } else {
        (None, None, None)
    }
}
