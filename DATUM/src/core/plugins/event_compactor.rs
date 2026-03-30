use std::collections::HashSet;

use anyhow::Result;
use serde_json::json;

use crate::core::ids;
use crate::core::model::{ActionRecord, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct EventCompactorPlugin;

impl EventCompactorPlugin {
    pub fn new() -> Self {
        Self
    }
}

fn severity_rank(sev: &str) -> u8 {
    match sev {
        "critical" => 4,
        "error" => 3,
        "warning" => 2,
        "info" => 1,
        _ => 0,
    }
}

fn event_key(ev: &Event) -> String {
    if ev.kind == "service_status_changed" {
        format!("{}|{}|{}|{}", ev.kind, ev.severity, ev.target, ev.message)
    } else {
        format!("{}|{}|{}", ev.kind, ev.severity, ev.target)
    }
}

fn action_key(a: &ActionRecord) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        a.correlation_id, a.phase, a.status, a.kind, a.service_id, a.target_id
    )
}

impl Plugin for EventCompactorPlugin {
    fn name(&self) -> &'static str {
        "event_compactor"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        let evcfg = cfg.agent.events.clone().unwrap_or_default();

        let max_per_snapshot = evcfg.max_per_snapshot.unwrap_or(50);
        let deduplicate = evcfg.deduplicate.unwrap_or(true);
        let prefer_severity = evcfg.prefer_severity.unwrap_or(true);

        let original_len = snapshot.events.len();

        if deduplicate {
            let mut seen: HashSet<String> = HashSet::new();
            let mut deduped: Vec<Event> = Vec::with_capacity(snapshot.events.len());

            for ev in snapshot.events.drain(..) {
                let key = event_key(&ev);
                if seen.insert(key) {
                    deduped.push(ev);
                }
            }

            snapshot.events = deduped;

            let mut action_seen: HashSet<String> = HashSet::new();
            let mut deduped_actions: Vec<ActionRecord> = Vec::with_capacity(snapshot.actions.len());
            for action in snapshot.actions.drain(..) {
                let key = action_key(&action);
                if action_seen.insert(key) {
                    deduped_actions.push(action);
                }
            }
            snapshot.actions = deduped_actions;
        }

        if prefer_severity {
            snapshot.events.sort_by(|a, b| {
                let sa = severity_rank(&a.severity);
                let sb = severity_rank(&b.severity);
                sb.cmp(&sa).then_with(|| a.id.cmp(&b.id))
            });
        }

        let compacted_len = snapshot.events.len();
        let mut truncated = false;

        if snapshot.events.len() > max_per_snapshot {
            snapshot.events.truncate(max_per_snapshot);
            truncated = true;
        }

        let final_len = snapshot.events.len();

        if truncated {
            snapshot.events.push(Event {
                id: format!(
                    "event:events_truncated:{}",
                    ids::sanitize_id_component(&snapshot.meta.cycle_id)
                ),
                kind: "events_truncated".to_string(),
                severity: "info".to_string(),
                target: "snapshot".to_string(),
                message: format!(
                    "Events truncated from {} to {} (original before dedup: {})",
                    compacted_len, final_len, original_len
                ),
                correlation_id: None,
                source_plugin: Some("event_compactor".to_string()),
                status: Some("resolved".to_string()),
                details: Some(json!({"original_len": original_len, "compacted_len": compacted_len, "final_len": final_len, "max_per_snapshot": max_per_snapshot})),
                observed_at_utc: snapshot.meta.observed_at_utc,
            });
        }

        Ok(())
    }
}
