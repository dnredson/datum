use anyhow::Result;
use serde_json::json;
use std::process::Command;

use crate::core::ids;
use crate::core::model::{Edge, Entity, Snapshot};
use crate::core::plugin::Plugin;

pub struct ConnectionPingLinuxPlugin;

impl ConnectionPingLinuxPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for ConnectionPingLinuxPlugin {
    fn name(&self) -> &'static str {
        "connection_ping_linux"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(unix) {
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
    rtt_min_ms: Option<f64>,
    rtt_avg_ms: Option<f64>,
    rtt_max_ms: Option<f64>,
    raw: String,
}

fn ping_and_parse(target: &str, count: u32, timeout_s: u32) -> PingResult {
    // Linux ping: -c <count> -W <timeout_s> (timeout for each reply, seconds)
    let out = Command::new("ping")
        .args([
            "-c",
            &count.to_string(),
            "-W",
            &timeout_s.to_string(),
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
            parse_linux_ping_output(&raw)
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

fn parse_linux_ping_output(raw: &str) -> PingResult {
    let lower = raw.to_lowercase();

    // "4 packets transmitted, 4 received, 0% packet loss, time 3004ms"
    let (received, loss_pct) = parse_loss(&lower);
    let available = received > 0 && loss_pct < 100;

    // "rtt min/avg/max/mdev = 0.522/0.601/0.728/0.070 ms"
    let (min_ms, avg_ms, max_ms) = parse_rtt_triplet(&lower);

    PingResult {
        available,
        loss_pct,
        rtt_min_ms: min_ms,
        rtt_avg_ms: avg_ms,
        rtt_max_ms: max_ms,
        raw: raw.trim().to_string(),
    }
}

fn parse_loss(s: &str) -> (u64, u64) {
    let mut received: u64 = 0;
    let mut loss_pct: u64 = 100;

    // Try to parse packet loss percentage
    if let Some(pct) = extract_first_percentage(s) {
        loss_pct = pct;
    }

    // Parse received count: "<tx> packets transmitted, <rx> received"
    if let Some(idx) = s.find("packets transmitted") {
        let prefix = &s[..idx];
        // find last number in prefix
        if let Some(tx) = extract_last_u64(prefix) {
            // now find "received" after idx
            if let Some(ridx) = s[idx..].find("received") {
                let mid = &s[idx..idx + ridx];
                received = extract_last_u64(mid).unwrap_or(0);
                // if parsing failed, fallback
                let _ = tx;
            }
        }
    } else if s.contains("received") {
        // fallback: find last number before "received"
        if let Some(pos) = s.find("received") {
            received = extract_last_u64(&s[..pos]).unwrap_or(0);
        }
    }

    (received, loss_pct)
}

fn parse_rtt_triplet(s: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let key = "rtt min/avg/max";
    if let Some(pos) = s.find(key) {
        let tail = &s[pos..];
        if let Some(eq) = tail.find('=') {
            let after = tail[eq + 1..].trim();
            // values until " ms"
            let vals_part = after.split(" ms").next().unwrap_or(after).trim();
            let nums: Vec<f64> = vals_part
                .split('/')
                .take(3)
                .filter_map(|x| x.trim().parse::<f64>().ok())
                .collect();
            if nums.len() == 3 {
                return (Some(nums[0]), Some(nums[1]), Some(nums[2]));
            }
        }
    }
    (None, None, None)
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

fn extract_last_u64(s: &str) -> Option<u64> {
    let mut last: Option<u64> = None;
    for part in s.split_whitespace() {
        if let Ok(v) = part.parse::<u64>() {
            last = Some(v);
        }
    }
    last
}
