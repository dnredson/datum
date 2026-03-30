use anyhow::Result;
use serde_json::{json, Value as JsonValue};
use std::time::Duration;

use crate::core::ids;
use crate::core::model::{ActionRecord, Entity, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct RevalidatorPlugin;
impl RevalidatorPlugin { pub fn new() -> Self { Self } }

fn mk_verification_id(cycle_id: &str, correlation_id: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let e = ids::sanitize_id_component(correlation_id);
    format!("recovery_verification:{}:{}", e, c)
}
fn mk_event_id(cycle_id: &str, correlation_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let e = ids::sanitize_id_component(correlation_id);
    let s = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", e, s, c)
}
fn run_check_http(spec: &JsonValue) -> Option<JsonValue> {
    let (kind, params) = if spec.get("params").is_some() { (spec.get("kind")?.as_str()?, spec.get("params")?) } else { (spec.get("kind")?.as_str()?, spec) };
    if kind != "http" && kind != "http_health" { return None; }
    let url = params.get("url")?.as_str()?;
    let timeout_ms = params.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(1500);
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_millis(timeout_ms)).build().ok()?;
    match client.get(url).send() {
        Ok(resp) => Some(json!({"kind":"http","status": if resp.status().is_success() {"available"} else {"unavailable"},"http_status":resp.status().as_u16()})),
        Err(_) => Some(json!({"kind":"http","status":"unavailable","reason":"request_failed"})),
    }
}
fn run_check_tcp(spec: &JsonValue) -> Option<JsonValue> {
    let (kind, params) = if spec.get("params").is_some() { (spec.get("kind")?.as_str()?, spec.get("params")?) } else { (spec.get("kind")?.as_str()?, spec) };
    if kind != "tcp" && kind != "tcp_port" { return None; }
    let host = params.get("host")?.as_str()?;
    let port = params.get("port")?.as_u64()?;
    let timeout_ms = params.get("timeout_ms").and_then(|v| v.as_u64()).unwrap_or(1000);
    let addr = format!("{}:{}", host, port);
    let parsed = addr.parse().ok()?;
    match std::net::TcpStream::connect_timeout(&parsed, Duration::from_millis(timeout_ms)) {
        Ok(_) => Some(json!({"kind":"tcp","status":"available"})),
        Err(_) => Some(json!({"kind":"tcp","status":"unavailable"})),
    }
}
fn run_check_process(snapshot: &Snapshot, spec: &JsonValue) -> Option<JsonValue> {
    let (kind, params) = if spec.get("params").is_some() { (spec.get("kind")?.as_str()?, spec.get("params")?) } else { (spec.get("kind")?.as_str()?, spec) };
    if kind != "process" && kind != "process_alive" { return None; }
    let name = params.get("name")?.as_str()?.to_lowercase();
    let found = snapshot.entities.iter().any(|e| e.kind == "process" && e.attrs.get("name").and_then(|v| v.as_str()).map(|n| n.to_lowercase().contains(&name)).unwrap_or(false));
    Some(json!({"kind":"process","status": if found {"available"} else {"unavailable"}}))
}
fn find_service<'a>(snapshot: &'a Snapshot, service_id: &str) -> Option<&'a Entity> { snapshot.entities.iter().find(|e| e.kind == "service" && e.id == service_id) }
fn check_declared_list(service: &Entity) -> Vec<JsonValue> { service.attrs.get("checks_declared").and_then(|v| v.as_array()).cloned().unwrap_or_default() }
fn revalidate_service(snapshot: &Snapshot, service: &Entity) -> (String, Vec<JsonValue>) {
    let checks_declared = check_declared_list(service);
    let mut results = Vec::new();
    for spec in &checks_declared {
        if let Some(r) = run_check_http(spec).or_else(|| run_check_tcp(spec)).or_else(|| run_check_process(snapshot, spec)) { results.push(r); }
    }
    let has_available = results.iter().any(|r| r.get("status").and_then(|v| v.as_str()) == Some("available"));
    let has_unavailable = results.iter().any(|r| r.get("status").and_then(|v| v.as_str()) == Some("unavailable"));
    let status = if results.is_empty() { "unknown".to_string() } else if has_available && !has_unavailable { "available".to_string() } else if has_available && has_unavailable { "degraded".to_string() } else { "unavailable".to_string() };
    (status, results)
}
fn finished_actions(snapshot: &Snapshot) -> Vec<ActionRecord> { snapshot.actions.iter().filter(|a| a.phase == "finished").cloned().collect() }
fn should_mark_pending(action: &ActionRecord, validation_status: &str) -> bool {
    if action.status != "succeeded" { return false; }
    if !matches!(action.kind.as_str(), "restart_command" | "start_container" | "restart_container") { return false; }
    matches!(validation_status, "unavailable" | "degraded" | "unknown")
}

impl Plugin for RevalidatorPlugin {
    fn name(&self) -> &'static str { "revalidator" }
    fn apply(&self, snapshot: &mut Snapshot, _cfg: &crate::config::Config) -> Result<()> {
        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();
        let actions = finished_actions(snapshot);
        if actions.is_empty() { return Ok(()); }
        let mut new_entities = Vec::new();
        let mut new_events = Vec::new();
        for action in actions {
            let service = match find_service(snapshot, &action.service_id) { Some(s) => s, None => continue };
            let (validation_status, checks_result) = revalidate_service(snapshot, service);
            let (verification_status, event_kind, severity, message) = if action.status == "dry_run" {
                ("dry_run", "recovery_dry_run", "info", format!("Recovery for service '{}' was dry-run only", action.service_name))
            } else if action.status == "skipped" || action.suppressed {
                ("skipped", "recovery_skipped", "info", format!("Recovery for service '{}' was skipped", action.service_name))
            } else if action.status == "failed" {
                ("not_effective", "recovery_not_effective", "error", format!("Recovery for service '{}' failed during execution", action.service_name))
            } else if validation_status == "available" {
                ("confirmed", "recovery_confirmed", "warning", format!("Recovery confirmed for service '{}'", action.service_name))
            } else if should_mark_pending(&action, &validation_status) {
                ("pending_observation", "recovery_pending_observation", "info", format!("Recovery for service '{}' executed, but confirmation should be observed in the next cycle", action.service_name))
            } else {
                ("not_effective", "recovery_not_effective", "error", format!("Recovery not effective for service '{}'; validation_status={}", action.service_name, validation_status))
            };
            new_entities.push(Entity {
                id: mk_verification_id(&cycle_id, &action.correlation_id),
                kind: "recovery_verification".to_string(),
                attrs: json!({
                    "correlation_id": action.correlation_id,
                    "origin": action.origin,
                    "service_id": action.service_id,
                    "service_name": action.service_name,
                    "action_kind": action.kind,
                    "action_status": action.status,
                    "verification_status": verification_status,
                    "validation_status": validation_status,
                    "checks_result": checks_result,
                }),
                observed_at_utc: observed_at,
                ttl_s: 60,
            });
            new_events.push(Event { id: mk_event_id(&cycle_id, &action.correlation_id, event_kind), kind: event_kind.to_string(), severity: severity.to_string(), target: action.service_id.clone(), message, observed_at_utc: observed_at });
        }
        snapshot.entities.extend(new_entities);
        snapshot.events.extend(new_events);
        Ok(())
    }
}
