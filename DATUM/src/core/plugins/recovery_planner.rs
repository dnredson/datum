use anyhow::Result;
use crate::config::{Config, ServiceRecoverySpec, ServiceRegistryService};
use crate::core::ids;
use crate::core::model::{ActionRecord, Entity, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct RecoveryPlannerPlugin;
impl RecoveryPlannerPlugin { pub fn new() -> Self { Self } }

#[derive(Debug, Clone)]
struct PlannedRecovery {
    service_id: String,
    service_name: String,
    service_status: String,
    action_kind: String,
    target_kind: String,
    target_id: String,
    reason: String,
    suppressed: bool,
    suppression_reason: Option<String>,
}

fn norm(s: &str) -> String { s.trim().to_lowercase() }
fn mk_correlation_id(service_name: &str, cycle_id: &str) -> String {
    let s = ids::sanitize_id_component(service_name); let c = ids::sanitize_id_component(cycle_id);
    format!("recovery:{}:{}", s, c)
}
fn mk_action_id(cycle_id: &str, correlation_id: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id); let x = ids::sanitize_id_component(correlation_id);
    format!("action:{}:planned:{}", x, c)
}
fn mk_event_id(cycle_id: &str, service_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id); let s = ids::sanitize_id_component(service_id); let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
}
fn is_service_allowed(service_name: &str, allow: &Option<Vec<String>>) -> bool {
    match allow { None => true, Some(list) if list.is_empty() => true, Some(list) => list.iter().any(|x| x.eq_ignore_ascii_case(service_name)) }
}
fn service_has_binding(attrs: &serde_json::Value) -> bool {
    attrs.get("binding").and_then(|b| b.get("bound")).and_then(|v| v.as_bool()).unwrap_or(false)
}
fn service_runtime(attrs: &serde_json::Value) -> Option<&str> { attrs.get("runtime").and_then(|v| v.as_str()) }
fn find_registry_service<'a>(cfg: &'a Config, service_name: &str) -> Option<&'a ServiceRegistryService> { cfg.find_registry_service(service_name) }
fn planned_action_exists(snapshot: &Snapshot, service_name: &str) -> bool {
    let wanted = norm(service_name);
    snapshot.actions.iter().any(|a| a.phase == "planned" && norm(&a.service_name) == wanted)
}
fn policy_matches_runtime(runtime: Option<&str>, policy: &str) -> bool {
    matches!((runtime.unwrap_or(""), policy),
        ("process", "restart_command") |
        ("container", "docker_restart") |
        ("container", "docker_start") |
        ("", "restart_container") |
        ("", "start_container") |
        ("", "restart_process"))
}
fn resolve_restart_command(_recovery: &ServiceRecoverySpec) -> bool { true }
fn plan_from_registry_policy(attrs: &serde_json::Value, svc: &ServiceRegistryService) -> Option<(String, String, String)> {
    let runtime = service_runtime(attrs); let recovery = svc.recovery.as_ref()?; let policy = recovery.policy.trim();
    if policy.is_empty() || !policy_matches_runtime(runtime, policy) { return None; }
    let bindings = attrs.get("bindings").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    match policy {
        "docker_restart" => {
            for b in &bindings {
                let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
                let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
                let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
                if bound && target_kind == "container" && !target_id.is_empty() { return Some(("restart_container".to_string(), "container".to_string(), target_id.to_string())); }
            }
            for b in &bindings {
                let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
                let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
                let via = b.get("via").and_then(|v| v.as_str()).unwrap_or("");
                let wanted = b.get("wanted").and_then(|v| v.as_str()).unwrap_or("").trim();
                if !bound && target_kind == "container" && via == "container_name" && !wanted.is_empty() { return Some(("start_container".to_string(), "container_name".to_string(), wanted.to_string())); }
            }
            None
        }
        "docker_start" => {
            for b in &bindings {
                let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
                let via = b.get("via").and_then(|v| v.as_str()).unwrap_or("");
                let wanted = b.get("wanted").and_then(|v| v.as_str()).unwrap_or("").trim();
                if target_kind == "container" && via == "container_name" && !wanted.is_empty() { return Some(("start_container".to_string(), "container_name".to_string(), wanted.to_string())); }
            }
            None
        }
        "restart_command" => {
            if resolve_restart_command(recovery) { Some(("restart_command".to_string(), "service".to_string(), svc.name.clone())) } else { None }
        }
        _ => None,
    }
}
fn choose_recovery_target_legacy(attrs: &serde_json::Value) -> Option<(String, String, String)> {
    let bindings = attrs.get("bindings").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for b in &bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if bound && target_kind == "container" && !target_id.is_empty() { return Some(("restart_container".to_string(), "container".to_string(), target_id.to_string())); }
    }
    for b in &bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let via = b.get("via").and_then(|v| v.as_str()).unwrap_or("");
        let wanted = b.get("wanted").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !bound && target_kind == "container" && via == "container_name" && !wanted.is_empty() { return Some(("start_container".to_string(), "container_name".to_string(), wanted.to_string())); }
    }
    for b in &bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if bound && target_kind == "process" && !target_id.is_empty() { return Some(("restart_process".to_string(), "process".to_string(), target_id.to_string())); }
    }
    None
}

impl Plugin for RecoveryPlannerPlugin {
    fn name(&self) -> &'static str { "recovery_planner" }
    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        let rcfg = match cfg.auto_recovery.as_ref() { Some(r) if r.enabled => r, _ => return Ok(()) };
        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();
        let dry_run = rcfg.dry_run.unwrap_or(true);
        let allow_services = rcfg.allow_services.clone();

        let service_views: Vec<(String, String, String, serde_json::Value)> = snapshot.entities.iter().filter(|e| e.kind == "service").map(|e| {
            let name = e.attrs.get("name").and_then(|v| v.as_str()).unwrap_or(&e.id).to_string();
            let status = e.attrs.get("status").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
            (e.id.clone(), name, status, e.attrs.clone())
        }).collect();

        let mut new_actions = Vec::new();
        let mut new_events = Vec::new();

        for (service_id, service_name, service_status, attrs) in service_views {
            if !matches!(service_status.as_str(), "degraded" | "unavailable") { continue; }
            if planned_action_exists(snapshot, &service_name) { continue; }
            if !is_service_allowed(&service_name, &allow_services) {
                new_events.push(Event { id: mk_event_id(&cycle_id, &service_id, "recovery_suppressed_not_allowed"), kind: "recovery_suppressed".to_string(), severity: "info".to_string(), target: service_id.clone(), message: format!("Recovery planning suppressed for service '{}' because it is not in allow_services", service_name), observed_at_utc: observed_at });
                continue;
            }

            let runtime = service_runtime(&attrs);
            let registry_service = find_registry_service(cfg, &service_name);
            let plan = if let Some(svc) = registry_service {
                if let Some(recovery) = svc.recovery.as_ref() {
                    if !policy_matches_runtime(runtime, recovery.policy.as_str()) {
                        PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind: "noop".to_string(), target_kind: "service".to_string(), target_id: service_id.clone(), reason: "recovery policy is incompatible with service runtime".to_string(), suppressed: true, suppression_reason: Some("recovery policy mismatch".to_string()) }
                    } else if let Some((action_kind, target_kind, target_id)) = plan_from_registry_policy(&attrs, svc) {
                        PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind, target_kind, target_id, reason: format!("service is {}", service_status), suppressed: false, suppression_reason: None }
                    } else if !service_has_binding(&attrs) {
                        PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind: "noop".to_string(), target_kind: "service".to_string(), target_id: service_id.clone(), reason: "service has no resolved binding".to_string(), suppressed: true, suppression_reason: Some("service is unbound".to_string()) }
                    } else {
                        PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind: "escalate".to_string(), target_kind: "service".to_string(), target_id: service_id.clone(), reason: "registry policy exists but planner could not infer a recovery target".to_string(), suppressed: true, suppression_reason: Some("planner could not infer a recovery target from registry policy".to_string()) }
                    }
                } else if let Some((action_kind, target_kind, target_id)) = choose_recovery_target_legacy(&attrs) {
                    PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind, target_kind, target_id, reason: format!("service is {}", service_status), suppressed: false, suppression_reason: None }
                } else {
                    PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind: "noop".to_string(), target_kind: "service".to_string(), target_id: service_id.clone(), reason: "service has no resolved binding".to_string(), suppressed: true, suppression_reason: Some("service is unbound".to_string()) }
                }
            } else if let Some((action_kind, target_kind, target_id)) = choose_recovery_target_legacy(&attrs) {
                PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind, target_kind, target_id, reason: format!("service is {}", service_status), suppressed: false, suppression_reason: None }
            } else {
                PlannedRecovery { service_id: service_id.clone(), service_name: service_name.clone(), service_status: service_status.clone(), action_kind: "noop".to_string(), target_kind: "service".to_string(), target_id: service_id.clone(), reason: "service has no resolved binding".to_string(), suppressed: true, suppression_reason: Some("service is unbound".to_string()) }
            };

            let correlation_id = mk_correlation_id(&service_name, &cycle_id);
            new_actions.push(ActionRecord {
                id: mk_action_id(&cycle_id, &correlation_id),
                correlation_id: correlation_id.clone(),
                origin: "recovery_planner".to_string(),
                kind: plan.action_kind.clone(),
                phase: "planned".to_string(),
                status: if plan.suppressed { "suppressed" } else { "pending" }.to_string(),
                service_id: plan.service_id.clone(),
                service_name: plan.service_name.clone(),
                target_kind: plan.target_kind.clone(),
                target_id: plan.target_id.clone(),
                reason: Some(plan.reason.clone()),
                message: plan.suppression_reason.clone(),
                dry_run,
                suppressed: plan.suppressed,
                exit_code: None,
                duration_ms: None,
                stdout: None,
                stderr: None,
                observed_at_utc: observed_at,
            });

            let (kind, severity, message) = if plan.suppressed {
                ("recovery_suppressed", "info", format!("Recovery plan suppressed for service '{}': {}", service_name, plan.suppression_reason.unwrap_or_else(|| "unknown reason".to_string())))
            } else {
                ("recovery_planned", "warning", format!("Recovery planned for service '{}': action={} target_kind={} target_id={} dry_run={}", service_name, plan.action_kind, plan.target_kind, plan.target_id, dry_run))
            };
            new_events.push(Event { id: mk_event_id(&cycle_id, &service_id, kind), kind: kind.to_string(), severity: severity.to_string(), target: service_id.clone(), message, observed_at_utc: observed_at });
        }

        snapshot.actions.extend(new_actions);
        snapshot.events.extend(new_events);
        Ok(())
    }
}
