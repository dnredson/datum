use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;

use crate::config::{Config, ServiceRecoverySpec, ServiceRegistryService};
use crate::core::ids;
use crate::core::model::{ActionRecord, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct RecoveryPlannerPlugin;
impl RecoveryPlannerPlugin {
    pub fn new() -> Self {
        Self
    }
}

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

#[derive(Debug, Clone)]
struct RecoveryHistory {
    last_attempt_at: Option<DateTime<Utc>>,
    attempts_in_window: usize,
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn mk_correlation_id(service_name: &str, cycle_id: &str) -> String {
    let s = ids::sanitize_id_component(service_name);
    let c = ids::sanitize_id_component(cycle_id);
    format!("recovery:{}:{}", s, c)
}

fn mk_action_id(cycle_id: &str, correlation_id: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let x = ids::sanitize_id_component(correlation_id);
    format!("action:{}:planned:{}", x, c)
}

fn mk_event_id(cycle_id: &str, service_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let s = ids::sanitize_id_component(service_id);
    let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
}

fn is_service_allowed(service_name: &str, allow: &Option<Vec<String>>) -> bool {
    match allow {
        None => true,
        Some(list) if list.is_empty() => true,
        Some(list) => list.iter().any(|x| x.eq_ignore_ascii_case(service_name)),
    }
}

fn service_has_binding(attrs: &serde_json::Value) -> bool {
    attrs
        .get("binding")
        .and_then(|b| b.get("bound"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn service_runtime(attrs: &serde_json::Value) -> Option<&str> {
    attrs.get("runtime").and_then(|v| v.as_str())
}

fn find_registry_service<'a>(cfg: &'a Config, service_name: &str) -> Option<&'a ServiceRegistryService> {
    cfg.find_registry_service(service_name)
}

fn planned_action_exists(snapshot: &Snapshot, service_name: &str) -> bool {
    let wanted = norm(service_name);
    snapshot.actions.iter().any(|a| {
        a.phase == "planned"
            && norm(&a.service_name) == wanted
            && a.origin == "recovery_planner"
    })
}

fn policy_matches_runtime(runtime: Option<&str>, policy: &str) -> bool {
    matches!(
        (runtime.unwrap_or(""), policy),
        ("process", "restart_command")
            | ("container", "docker_restart")
            | ("container", "docker_start")
            | ("", "restart_container")
            | ("", "start_container")
            | ("", "restart_process")
    )
}

fn resolve_restart_command(_recovery: &ServiceRecoverySpec) -> bool {
    true
}

fn plan_from_registry_policy(
    attrs: &serde_json::Value,
    svc: &ServiceRegistryService,
) -> Option<(String, String, String)> {
    let runtime = service_runtime(attrs);
    let recovery = svc.recovery.as_ref()?;
    let policy = recovery.policy.trim();
    if policy.is_empty() || !policy_matches_runtime(runtime, policy) {
        return None;
    }

    let bindings = attrs
        .get("bindings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    match policy {
        "docker_restart" => {
            for b in &bindings {
                let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
                let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
                let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
                if bound && target_kind == "container" && !target_id.is_empty() {
                    return Some((
                        "restart_container".to_string(),
                        "container".to_string(),
                        target_id.to_string(),
                    ));
                }
            }
            None
        }
        "docker_start" => {
            for b in &bindings {
                let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
                let via = b.get("via").and_then(|v| v.as_str()).unwrap_or("");
                let wanted = b
                    .get("wanted")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if target_kind == "container" && via == "container_name" && !wanted.is_empty() {
                    return Some((
                        "start_container".to_string(),
                        "container_name".to_string(),
                        wanted.to_string(),
                    ));
                }
            }
            None
        }
        "restart_command" => {
            if resolve_restart_command(recovery) {
                Some((
                    "restart_command".to_string(),
                    "service".to_string(),
                    svc.name.clone(),
                ))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn choose_recovery_target_legacy(attrs: &serde_json::Value) -> Option<(String, String, String)> {
    let bindings = attrs
        .get("bindings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for b in &bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if bound && target_kind == "container" && !target_id.is_empty() {
            return Some((
                "restart_container".to_string(),
                "container".to_string(),
                target_id.to_string(),
            ));
        }
    }

    for b in &bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let via = b.get("via").and_then(|v| v.as_str()).unwrap_or("");
        let wanted = b
            .get("wanted")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if !bound && target_kind == "container" && via == "container_name" && !wanted.is_empty() {
            return Some((
                "start_container".to_string(),
                "container_name".to_string(),
                wanted.to_string(),
            ));
        }
    }

    for b in &bindings {
        let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if bound && target_kind == "process" && !target_id.is_empty() {
            return Some((
                "restart_process".to_string(),
                "process".to_string(),
                target_id.to_string(),
            ));
        }
    }

    None
}

fn load_previous_snapshot(cfg: &Config) -> Option<Snapshot> {
    let file_cfg = cfg.outputs.file_json.as_ref()?;
    if !file_cfg.enabled {
        return None;
    }
    let raw = fs::read_to_string(&file_cfg.path).ok()?;
    serde_json::from_str::<Snapshot>(&raw).ok()
}

fn collect_recovery_history(
    previous: Option<&Snapshot>,
    service_name: &str,
    observed_at: DateTime<Utc>,
    window_s: u64,
) -> RecoveryHistory {
    let Some(prev) = previous else {
        return RecoveryHistory {
            last_attempt_at: None,
            attempts_in_window: 0,
        };
    };

    let wanted = norm(service_name);
    let mut correlations_in_window = BTreeSet::new();
    let mut last_attempt_at: Option<DateTime<Utc>> = None;

    for action in &prev.actions {
        if norm(&action.service_name) != wanted {
            continue;
        }
        if action.origin != "recovery_planner" {
            continue;
        }
        if action.suppressed {
            continue;
        }

        let ts = action.observed_at_utc;
        if last_attempt_at.map(|x| ts > x).unwrap_or(true) {
            last_attempt_at = Some(ts);
        }

        let age_s = observed_at.signed_duration_since(ts).num_seconds();
        if age_s >= 0 && (age_s as u64) <= window_s {
            correlations_in_window.insert(action.correlation_id.clone());
        }
    }

    RecoveryHistory {
        last_attempt_at,
        attempts_in_window: correlations_in_window.len(),
    }
}

fn make_suppressed_action(
    cycle_id: &str,
    correlation_id: &str,
    observed_at: DateTime<Utc>,
    dry_run: bool,
    plan: &PlannedRecovery,
    message: Option<String>,
) -> ActionRecord {
    ActionRecord {
        id: mk_action_id(cycle_id, correlation_id),
        correlation_id: correlation_id.to_string(),
        origin: "recovery_planner".to_string(),
        kind: plan.action_kind.clone(),
        phase: "planned".to_string(),
        status: "suppressed".to_string(),
        service_id: plan.service_id.clone(),
        service_name: plan.service_name.clone(),
        target_kind: plan.target_kind.clone(),
        target_id: plan.target_id.clone(),
        reason: Some(plan.reason.clone()),
        message,
        dry_run,
        suppressed: true,
        exit_code: None,
        duration_ms: None,
        stdout: None,
        stderr: None,
        observed_at_utc: observed_at,
    }
}

impl Plugin for RecoveryPlannerPlugin {
    fn name(&self) -> &'static str {
        "recovery_planner"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        let rcfg = match cfg.auto_recovery.as_ref() {
            Some(r) if r.enabled => r,
            _ => return Ok(()),
        };

        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();
        let dry_run = rcfg.dry_run.unwrap_or(true);
        let allow_services = rcfg.allow_services.clone();
        let cooldown_s = rcfg.cooldown_s.unwrap_or(60);
        let max_attempts_per_window = rcfg.max_attempts_per_window.unwrap_or(3) as usize;
        let attempt_window_s = rcfg.attempt_window_s.unwrap_or(300);
        let previous_snapshot = load_previous_snapshot(cfg);

        let service_views: Vec<(String, String, String, serde_json::Value)> = snapshot
            .entities
            .iter()
            .filter(|e| e.kind == "service")
            .map(|e| {
                let name = e
                    .attrs
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&e.id)
                    .to_string();
                let status = e
                    .attrs
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                (e.id.clone(), name, status, e.attrs.clone())
            })
            .collect();

        let mut new_actions = Vec::new();
        let mut new_events = Vec::new();

        for (service_id, service_name, service_status, attrs) in service_views {
            if !matches!(service_status.as_str(), "degraded" | "unavailable") {
                continue;
            }
            if planned_action_exists(snapshot, &service_name) {
                continue;
            }

            let correlation_id = mk_correlation_id(&service_name, &cycle_id);

            if !is_service_allowed(&service_name, &allow_services) {
                let plan = PlannedRecovery {
                    service_id: service_id.clone(),
                    service_name: service_name.clone(),
                    service_status: service_status.clone(),
                    action_kind: "noop".to_string(),
                    target_kind: "service".to_string(),
                    target_id: service_id.clone(),
                    reason: format!("service is {}", service_status),
                    suppressed: true,
                    suppression_reason: Some("service is not in allow_services".to_string()),
                };
                new_actions.push(make_suppressed_action(
                    &cycle_id,
                    &correlation_id,
                    observed_at,
                    dry_run,
                    &plan,
                    plan.suppression_reason.clone(),
                ));
                new_events.push(Event {
                    id: mk_event_id(&cycle_id, &service_id, "recovery_suppressed_not_allowed"),
                    kind: "recovery_suppressed_not_allowed".to_string(),
                    severity: "info".to_string(),
                    target: service_id.clone(),
                    message: format!(
                        "Recovery planning suppressed for service '{}' because it is not in allow_services",
                        service_name
                    ),
                    correlation_id: Some(correlation_id),
                    source_plugin: Some("recovery_planner".to_string()),
                    status: Some("suppressed".to_string()),
                    details: Some(json!({
                        "service_name": service_name,
                        "service_status": service_status,
                        "allow_services": allow_services,
                        "suppression_reason": "service is not in allow_services",
                        "dry_run": dry_run
                    })),
                    observed_at_utc: observed_at,
                });
                continue;
            }

            let history = collect_recovery_history(
                previous_snapshot.as_ref(),
                &service_name,
                observed_at,
                attempt_window_s,
            );

            if let Some(last_attempt_at) = history.last_attempt_at {
                let elapsed_s = observed_at
                    .signed_duration_since(last_attempt_at)
                    .num_seconds()
                    .max(0) as u64;
                if elapsed_s < cooldown_s {
                    let suppression_reason = format!(
                        "cooldown active: last recovery attempt was {}s ago and cooldown is {}s",
                        elapsed_s, cooldown_s
                    );
                    let plan = PlannedRecovery {
                        service_id: service_id.clone(),
                        service_name: service_name.clone(),
                        service_status: service_status.clone(),
                        action_kind: "noop".to_string(),
                        target_kind: "service".to_string(),
                        target_id: service_id.clone(),
                        reason: format!("service is {}", service_status),
                        suppressed: true,
                        suppression_reason: Some(suppression_reason.clone()),
                    };
                    new_actions.push(make_suppressed_action(
                        &cycle_id,
                        &correlation_id,
                        observed_at,
                        dry_run,
                        &plan,
                        Some(suppression_reason.clone()),
                    ));
                    new_events.push(Event {
                        id: mk_event_id(&cycle_id, &service_id, "recovery_suppressed_cooldown"),
                        kind: "recovery_suppressed_cooldown".to_string(),
                        severity: "info".to_string(),
                        target: service_id.clone(),
                        message: format!(
                            "Recovery suppressed for service '{}': cooldown active ({}s < {}s)",
                            service_name, elapsed_s, cooldown_s
                        ),
                        correlation_id: Some(correlation_id),
                        source_plugin: Some("recovery_planner".to_string()),
                        status: Some("suppressed".to_string()),
                        details: Some(json!({
                            "service_name": service_name,
                            "service_status": service_status,
                            "cooldown_s": cooldown_s,
                            "seconds_since_last_attempt": elapsed_s,
                            "last_attempt_at_utc": last_attempt_at,
                            "attempt_window_s": attempt_window_s,
                            "attempts_in_window": history.attempts_in_window,
                            "max_attempts_per_window": max_attempts_per_window,
                            "suppression_reason": suppression_reason,
                            "dry_run": dry_run
                        })),
                        observed_at_utc: observed_at,
                    });
                    continue;
                }
            }

            if history.attempts_in_window >= max_attempts_per_window {
                let suppression_reason = format!(
                    "attempt window exceeded: {} attempts in last {}s (max={})",
                    history.attempts_in_window, attempt_window_s, max_attempts_per_window
                );
                let plan = PlannedRecovery {
                    service_id: service_id.clone(),
                    service_name: service_name.clone(),
                    service_status: service_status.clone(),
                    action_kind: "noop".to_string(),
                    target_kind: "service".to_string(),
                    target_id: service_id.clone(),
                    reason: format!("service is {}", service_status),
                    suppressed: true,
                    suppression_reason: Some(suppression_reason.clone()),
                };
                new_actions.push(make_suppressed_action(
                    &cycle_id,
                    &correlation_id,
                    observed_at,
                    dry_run,
                    &plan,
                    Some(suppression_reason.clone()),
                ));
                new_events.push(Event {
                    id: mk_event_id(
                        &cycle_id,
                        &service_id,
                        "recovery_suppressed_attempt_window",
                    ),
                    kind: "recovery_suppressed_attempt_window".to_string(),
                    severity: "warning".to_string(),
                    target: service_id.clone(),
                    message: format!(
                        "Recovery suppressed for service '{}': too many attempts in time window",
                        service_name
                    ),
                    correlation_id: Some(correlation_id),
                    source_plugin: Some("recovery_planner".to_string()),
                    status: Some("suppressed".to_string()),
                    details: Some(json!({
                        "service_name": service_name,
                        "service_status": service_status,
                        "cooldown_s": cooldown_s,
                        "attempt_window_s": attempt_window_s,
                        "attempts_in_window": history.attempts_in_window,
                        "max_attempts_per_window": max_attempts_per_window,
                        "last_attempt_at_utc": history.last_attempt_at,
                        "suppression_reason": suppression_reason,
                        "dry_run": dry_run
                    })),
                    observed_at_utc: observed_at,
                });
                continue;
            }

            let runtime = service_runtime(&attrs);
            let registry_service = find_registry_service(cfg, &service_name);
            let plan = if let Some(svc) = registry_service {
                if let Some(recovery) = svc.recovery.as_ref() {
                    if !policy_matches_runtime(runtime, recovery.policy.as_str()) {
                        PlannedRecovery {
                            service_id: service_id.clone(),
                            service_name: service_name.clone(),
                            service_status: service_status.clone(),
                            action_kind: "noop".to_string(),
                            target_kind: "service".to_string(),
                            target_id: service_id.clone(),
                            reason: "recovery policy is incompatible with service runtime".to_string(),
                            suppressed: true,
                            suppression_reason: Some("recovery policy mismatch".to_string()),
                        }
                    } else if let Some((action_kind, target_kind, target_id)) =
                        plan_from_registry_policy(&attrs, svc)
                    {
                        PlannedRecovery {
                            service_id: service_id.clone(),
                            service_name: service_name.clone(),
                            service_status: service_status.clone(),
                            action_kind,
                            target_kind,
                            target_id,
                            reason: format!("service is {}", service_status),
                            suppressed: false,
                            suppression_reason: None,
                        }
                    } else if !service_has_binding(&attrs) {
                        PlannedRecovery {
                            service_id: service_id.clone(),
                            service_name: service_name.clone(),
                            service_status: service_status.clone(),
                            action_kind: "noop".to_string(),
                            target_kind: "service".to_string(),
                            target_id: service_id.clone(),
                            reason: "service has no resolved binding".to_string(),
                            suppressed: true,
                            suppression_reason: Some("service is unbound".to_string()),
                        }
                    } else {
                        PlannedRecovery {
                            service_id: service_id.clone(),
                            service_name: service_name.clone(),
                            service_status: service_status.clone(),
                            action_kind: "escalate".to_string(),
                            target_kind: "service".to_string(),
                            target_id: service_id.clone(),
                            reason:
                                "registry policy exists but planner could not infer a recovery target"
                                    .to_string(),
                            suppressed: true,
                            suppression_reason: Some(
                                "planner could not infer a recovery target from registry policy"
                                    .to_string(),
                            ),
                        }
                    }
                } else if let Some((action_kind, target_kind, target_id)) =
                    choose_recovery_target_legacy(&attrs)
                {
                    PlannedRecovery {
                        service_id: service_id.clone(),
                        service_name: service_name.clone(),
                        service_status: service_status.clone(),
                        action_kind,
                        target_kind,
                        target_id,
                        reason: format!("service is {}", service_status),
                        suppressed: false,
                        suppression_reason: None,
                    }
                } else {
                    PlannedRecovery {
                        service_id: service_id.clone(),
                        service_name: service_name.clone(),
                        service_status: service_status.clone(),
                        action_kind: "noop".to_string(),
                        target_kind: "service".to_string(),
                        target_id: service_id.clone(),
                        reason: "service has no resolved binding".to_string(),
                        suppressed: true,
                        suppression_reason: Some("service is unbound".to_string()),
                    }
                }
            } else if let Some((action_kind, target_kind, target_id)) =
                choose_recovery_target_legacy(&attrs)
            {
                PlannedRecovery {
                    service_id: service_id.clone(),
                    service_name: service_name.clone(),
                    service_status: service_status.clone(),
                    action_kind,
                    target_kind,
                    target_id,
                    reason: format!("service is {}", service_status),
                    suppressed: false,
                    suppression_reason: None,
                }
            } else {
                PlannedRecovery {
                    service_id: service_id.clone(),
                    service_name: service_name.clone(),
                    service_status: service_status.clone(),
                    action_kind: "noop".to_string(),
                    target_kind: "service".to_string(),
                    target_id: service_id.clone(),
                    reason: "service has no resolved binding".to_string(),
                    suppressed: true,
                    suppression_reason: Some("service is unbound".to_string()),
                }
            };

            if plan.suppressed {
                let suppression_reason_for_message = plan
                    .suppression_reason
                    .clone()
                    .unwrap_or_else(|| "unknown reason".to_string());
                let suppression_reason_for_details = plan.suppression_reason.clone();

                new_actions.push(make_suppressed_action(
                    &cycle_id,
                    &correlation_id,
                    observed_at,
                    dry_run,
                    &plan,
                    plan.suppression_reason.clone(),
                ));

                new_events.push(Event {
                    id: mk_event_id(&cycle_id, &service_id, "recovery_suppressed"),
                    kind: "recovery_suppressed".to_string(),
                    severity: "info".to_string(),
                    target: plan.service_id.clone(),
                    message: format!(
                        "Recovery plan suppressed for service '{}': {}",
                        plan.service_name, suppression_reason_for_message
                    ),
                    correlation_id: Some(correlation_id),
                    source_plugin: Some("recovery_planner".to_string()),
                    status: Some("suppressed".to_string()),
                    details: Some(json!({
                        "service_name": plan.service_name,
                        "service_status": plan.service_status,
                        "action_kind": plan.action_kind,
                        "target_kind": plan.target_kind,
                        "target_id": plan.target_id,
                        "reason": plan.reason,
                        "suppressed": true,
                        "suppression_reason": suppression_reason_for_details,
                        "cooldown_s": cooldown_s,
                        "attempt_window_s": attempt_window_s,
                        "attempts_in_window": history.attempts_in_window,
                        "max_attempts_per_window": max_attempts_per_window,
                        "last_attempt_at_utc": history.last_attempt_at,
                        "dry_run": dry_run
                    })),
                    observed_at_utc: observed_at,
                });
            } else {
                new_actions.push(ActionRecord {
                    id: mk_action_id(&cycle_id, &correlation_id),
                    correlation_id: correlation_id.clone(),
                    origin: "recovery_planner".to_string(),
                    kind: plan.action_kind.clone(),
                    phase: "planned".to_string(),
                    status: "pending".to_string(),
                    service_id: plan.service_id.clone(),
                    service_name: plan.service_name.clone(),
                    target_kind: plan.target_kind.clone(),
                    target_id: plan.target_id.clone(),
                    reason: Some(plan.reason.clone()),
                    message: None,
                    dry_run,
                    suppressed: false,
                    exit_code: None,
                    duration_ms: None,
                    stdout: None,
                    stderr: None,
                    observed_at_utc: observed_at,
                });

                new_events.push(Event {
                    id: mk_event_id(&cycle_id, &service_id, "recovery_planned"),
                    kind: "recovery_planned".to_string(),
                    severity: "warning".to_string(),
                    target: plan.service_id.clone(),
                    message: format!(
                        "Recovery planned for service '{}': action={} target_kind={} target_id={} dry_run={}",
                        plan.service_name, plan.action_kind, plan.target_kind, plan.target_id, dry_run
                    ),
                    correlation_id: Some(correlation_id),
                    source_plugin: Some("recovery_planner".to_string()),
                    status: Some("planned".to_string()),
                    details: Some(json!({
                        "service_name": plan.service_name,
                        "service_status": plan.service_status,
                        "action_kind": plan.action_kind,
                        "target_kind": plan.target_kind,
                        "target_id": plan.target_id,
                        "reason": plan.reason,
                        "cooldown_s": cooldown_s,
                        "attempt_window_s": attempt_window_s,
                        "attempts_in_window": history.attempts_in_window,
                        "max_attempts_per_window": max_attempts_per_window,
                        "last_attempt_at_utc": history.last_attempt_at,
                        "dry_run": dry_run
                    })),
                    observed_at_utc: observed_at,
                });
            }
        }

        snapshot.actions.extend(new_actions);
        snapshot.events.extend(new_events);
        Ok(())
    }
}
