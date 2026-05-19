use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::config::{Config, ServiceRegistryService};
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

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn service_entity<'a>(
    snapshot: &'a Snapshot,
    name: &str,
) -> Option<&'a crate::core::model::Entity> {
    let wanted = norm(name);
    snapshot.entities.iter().find(|e| {
        e.kind == "service"
            && e.attrs
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| norm(s) == wanted)
                .unwrap_or(false)
    })
}

fn service_id(snapshot: &Snapshot, name: &str) -> String {
    service_entity(snapshot, name)
        .map(|e| e.id.clone())
        .unwrap_or_else(|| ids::service_id(name))
}

fn service_status(snapshot: &Snapshot, name: &str) -> String {
    service_entity(snapshot, name)
        .and_then(|e| e.attrs.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn service_runtime(snapshot: &Snapshot, name: &str) -> Option<String> {
    service_entity(snapshot, name)
        .and_then(|e| e.attrs.get("runtime"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn service_has_binding(snapshot: &Snapshot, name: &str) -> bool {
    service_entity(snapshot, name)
        .and_then(|e| e.attrs.get("binding"))
        .and_then(|b| b.get("bound"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn declared_container_name(snapshot: &Snapshot, name: &str) -> Option<String> {
    let bindings = service_entity(snapshot, name)?
        .attrs
        .get("bindings")
        .and_then(|v| v.as_array())?;

    bindings.iter().find_map(|b| {
        let target_kind = b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
        let via = b.get("via").and_then(|v| v.as_str()).unwrap_or("");
        let wanted = b
            .get("wanted")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        if target_kind == "container" && via == "container_name" && !wanted.is_empty() {
            Some(wanted.to_string())
        } else {
            None
        }
    })
}

fn is_remediation_origin(origin: &str) -> bool {
    origin == "recovery_planner" || origin == "desired_state_planner"
}

fn is_real_attempt(action: &ActionRecord) -> bool {
    if action.suppressed {
        return false;
    }

    if !is_remediation_origin(&action.origin) {
        return false;
    }

    matches!(action.phase.as_str(), "planned" | "executing" | "finished")
}

fn existing_open_remediation(snapshot: &Snapshot, service_name: &str) -> bool {
    let wanted = norm(service_name);

    snapshot.actions.iter().any(|a| {
        norm(&a.service_name) == wanted
            && !a.suppressed
            && is_remediation_origin(&a.origin)
            && matches!(a.phase.as_str(), "planned" | "executing")
    })
}

fn find_recovery_candidate_services(snapshot: &Snapshot) -> Vec<String> {
    let mut out = Vec::new();

    for e in &snapshot.entities {
        if e.kind != "service" {
            continue;
        }

        let Some(name) = e.attrs.get("name").and_then(|v| v.as_str()) else {
            continue;
        };

        let status = e
            .attrs
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if matches!(status, "unavailable" | "degraded" | "failed" | "missing") {
            out.push(name.to_string());
        }
    }

    out.sort();
    out.dedup();
    out
}

fn policy_matches_runtime(runtime: &str, policy: &str) -> bool {
    matches!(
        (runtime, policy),
        ("process", "restart_command")
            | ("container", "docker_restart")
            | ("container", "docker_start")
    )
}

fn plan_from_registry(
    snapshot: &Snapshot,
    service_name: &str,
    svc: &ServiceRegistryService,
) -> PlannedRecovery {
    let sid = service_id(snapshot, service_name);
    let sstatus = service_status(snapshot, service_name);
    let runtime = service_runtime(snapshot, service_name).unwrap_or_else(|| svc.runtime.clone());

    let Some(recovery) = svc.recovery.as_ref() else {
        return PlannedRecovery {
            service_id: sid.clone(),
            service_name: service_name.to_string(),
            service_status: sstatus.clone(),
            action_kind: "noop".to_string(),
            target_kind: "service".to_string(),
            target_id: sid,
            reason: format!("service is {}", sstatus),
            suppressed: true,
            suppression_reason: Some("service has no recovery policy".to_string()),
        };
    };

    let policy = recovery.policy.trim();

    if policy.is_empty() {
        return PlannedRecovery {
            service_id: sid.clone(),
            service_name: service_name.to_string(),
            service_status: sstatus.clone(),
            action_kind: "noop".to_string(),
            target_kind: "service".to_string(),
            target_id: sid,
            reason: format!("service is {}", sstatus),
            suppressed: true,
            suppression_reason: Some("service has empty recovery policy".to_string()),
        };
    }

    if !policy_matches_runtime(&runtime, policy) {
        return PlannedRecovery {
            service_id: sid.clone(),
            service_name: service_name.to_string(),
            service_status: sstatus.clone(),
            action_kind: "noop".to_string(),
            target_kind: "service".to_string(),
            target_id: sid,
            reason: format!("service is {}", sstatus),
            suppressed: true,
            suppression_reason: Some(format!(
                "recovery policy '{}' is incompatible with runtime '{}'",
                policy, runtime
            )),
        };
    }

    match policy {
        "restart_command" => PlannedRecovery {
            service_id: sid.clone(),
            service_name: service_name.to_string(),
            service_status: sstatus.clone(),
            action_kind: "restart_command".to_string(),
            target_kind: "service".to_string(),
            target_id: sid,
            reason: format!("service is {}", sstatus),
            suppressed: false,
            suppression_reason: None,
        },
        "docker_restart" => {
            if service_has_binding(snapshot, service_name) {
                let tid = service_entity(snapshot, service_name)
                    .and_then(|e| e.attrs.get("bindings"))
                    .and_then(|v| v.as_array())
                    .and_then(|bindings| {
                        bindings.iter().find_map(|b| {
                            let bound = b.get("bound").and_then(|v| v.as_bool()).unwrap_or(false);
                            let target_kind =
                                b.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
                            let target_id =
                                b.get("target_id").and_then(|v| v.as_str()).unwrap_or("");

                            if bound && target_kind == "container" && !target_id.is_empty() {
                                Some(target_id.to_string())
                            } else {
                                None
                            }
                        })
                    });

                if let Some(target_id) = tid {
                    PlannedRecovery {
                        service_id: sid.clone(),
                        service_name: service_name.to_string(),
                        service_status: sstatus.clone(),
                        action_kind: "restart_container".to_string(),
                        target_kind: "container".to_string(),
                        target_id,
                        reason: format!("service is {}", sstatus),
                        suppressed: false,
                        suppression_reason: None,
                    }
                } else {
                    PlannedRecovery {
                        service_id: sid.clone(),
                        service_name: service_name.to_string(),
                        service_status: sstatus.clone(),
                        action_kind: "noop".to_string(),
                        target_kind: "service".to_string(),
                        target_id: sid,
                        reason: format!("service is {}", sstatus),
                        suppressed: true,
                        suppression_reason: Some(
                            "service has binding metadata but no bound container target"
                                .to_string(),
                        ),
                    }
                }
            } else {
                PlannedRecovery {
                    service_id: sid.clone(),
                    service_name: service_name.to_string(),
                    service_status: sstatus.clone(),
                    action_kind: "noop".to_string(),
                    target_kind: "service".to_string(),
                    target_id: sid,
                    reason: format!("service is {}", sstatus),
                    suppressed: true,
                    suppression_reason: Some("service is unbound".to_string()),
                }
            }
        }
        "docker_start" => {
            if let Some(container_name) = declared_container_name(snapshot, service_name) {
                PlannedRecovery {
                    service_id: sid.clone(),
                    service_name: service_name.to_string(),
                    service_status: sstatus.clone(),
                    action_kind: "start_container".to_string(),
                    target_kind: "container_name".to_string(),
                    target_id: container_name,
                    reason: format!("service is {}", sstatus),
                    suppressed: false,
                    suppression_reason: None,
                }
            } else {
                PlannedRecovery {
                    service_id: sid.clone(),
                    service_name: service_name.to_string(),
                    service_status: sstatus.clone(),
                    action_kind: "noop".to_string(),
                    target_kind: "service".to_string(),
                    target_id: sid,
                    reason: format!("service is {}", sstatus),
                    suppressed: true,
                    suppression_reason: Some(
                        "service has no declared container_name binding".to_string(),
                    ),
                }
            }
        }
        _ => PlannedRecovery {
            service_id: sid.clone(),
            service_name: service_name.to_string(),
            service_status: sstatus.clone(),
            action_kind: "noop".to_string(),
            target_kind: "service".to_string(),
            target_id: sid,
            reason: format!("service is {}", sstatus),
            suppressed: true,
            suppression_reason: Some(format!("unsupported recovery policy '{}'", policy)),
        },
    }
}

fn mk_correlation_id(service_name: &str, cycle_id: &str) -> String {
    let s = ids::sanitize_id_component(service_name);
    let c = ids::sanitize_id_component(cycle_id);
    format!("recovery:{}:{}", s, c)
}

fn mk_action_id(correlation_id: &str, cycle_id: &str) -> String {
    let x = ids::sanitize_id_component(correlation_id);
    let c = ids::sanitize_id_component(cycle_id);
    format!("action:{}:planned:{}", x, c)
}

fn mk_event_id(service_name: &str, suffix: &str, cycle_id: &str) -> String {
    let s = ids::sanitize_id_component(service_name);
    let x = ids::sanitize_id_component(suffix);
    let c = ids::sanitize_id_component(cycle_id);
    format!("event:{}:{}:{}", s, x, c)
}

fn snapshot_history_path(cfg: &Config) -> Option<PathBuf> {
    let file_json = cfg.outputs.file_json.as_ref()?;
    Some(PathBuf::from(file_json.path.clone()))
}

fn load_previous_snapshot(cfg: &Config) -> Option<Snapshot> {
    let path = snapshot_history_path(cfg)?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Snapshot>(&text).ok()
}

fn collect_attempt_history(
    snapshot: Option<&Snapshot>,
    service_name: &str,
    now: DateTime<Utc>,
    attempt_window_s: i64,
) -> (Option<DateTime<Utc>>, usize) {
    let Some(snapshot) = snapshot else {
        return (None, 0);
    };

    let wanted = norm(service_name);
    let mut last_attempt_at: Option<DateTime<Utc>> = None;
    let mut attempts_in_window = 0usize;

    for action in &snapshot.actions {
        if norm(&action.service_name) != wanted {
            continue;
        }

        if !is_real_attempt(action) {
            continue;
        }

        let ts = action.observed_at_utc;

        if last_attempt_at.map(|prev| ts > prev).unwrap_or(true) {
            last_attempt_at = Some(ts);
        }

        let age_s = now.signed_duration_since(ts).num_seconds();
        if age_s >= 0 && age_s <= attempt_window_s {
            attempts_in_window += 1;
        }
    }

    (last_attempt_at, attempts_in_window)
}

impl Plugin for RecoveryPlannerPlugin {
    fn name(&self) -> &'static str {
        "recovery_planner"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &Config) -> Result<()> {
        let rcfg = match cfg.auto_recovery.as_ref() {
            Some(r) if r.enabled => r,
            _ => return Ok(()),
        };

        let dry_run = rcfg.dry_run.unwrap_or(true);
        let cooldown_s = rcfg.cooldown_s.unwrap_or(60) as i64;
        let max_attempts_per_window = rcfg.max_attempts_per_window.unwrap_or(3) as usize;
        let attempt_window_s = rcfg.attempt_window_s.unwrap_or(300) as i64;

        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();

        let previous_snapshot = load_previous_snapshot(cfg);

        let mut new_actions = Vec::new();
        let mut new_events = Vec::new();

        for service_name in find_recovery_candidate_services(snapshot) {
            if existing_open_remediation(snapshot, &service_name) {
                continue;
            }

            let Some(registry_service) = cfg.find_registry_service(&service_name) else {
                let sid = service_id(snapshot, &service_name);
                new_events.push(Event {
                    id: mk_event_id(&service_name, "recovery_missing_registry", &cycle_id),
                    kind: "recovery_missing_registry".to_string(),
                    severity: "warning".to_string(),
                    target: sid,
                    message: format!(
                        "recovery planning skipped for service '{}' because it is not in service_registry",
                        service_name
                    ),
                    correlation_id: None,
                    source_plugin: Some("recovery_planner".to_string()),
                    status: Some("open".to_string()),
                    details: Some(json!({
                        "service_name": service_name,
                        "reason": "missing service_registry entry"
                    })),
                    observed_at_utc: observed_at,
                });
                continue;
            };

            let sid = service_id(snapshot, &service_name);
            let correlation_id = mk_correlation_id(&service_name, &cycle_id);

            let (last_attempt_at, attempts_in_window) = collect_attempt_history(
                previous_snapshot.as_ref(),
                &service_name,
                observed_at,
                attempt_window_s,
            );

            let seconds_since_last_attempt =
                last_attempt_at.map(|ts| observed_at.signed_duration_since(ts).num_seconds());

            if let Some(delta_s) = seconds_since_last_attempt {
                if delta_s >= 0 && delta_s < cooldown_s {
                    let reason = format!(
                        "cooldown active: last recovery attempt for '{}' happened {}s ago (cooldown={}s)",
                        service_name, delta_s, cooldown_s
                    );

                    let action = ActionRecord {
                        id: mk_action_id(&correlation_id, &cycle_id),
                        correlation_id: correlation_id.clone(),
                        origin: "recovery_planner".to_string(),
                        kind: "noop".to_string(),
                        phase: "planned".to_string(),
                        status: "suppressed".to_string(),
                        service_id: sid.clone(),
                        service_name: service_name.clone(),
                        target_kind: "service".to_string(),
                        target_id: sid.clone(),
                        reason: Some("recovery cooldown suppression".to_string()),
                        message: Some(reason.clone()),
                        dry_run,
                        suppressed: true,
                        exit_code: None,
                        duration_ms: None,
                        stdout: None,
                        stderr: None,
                        observed_at_utc: observed_at,
                    };

                    new_actions.push(action);

                    new_events.push(Event {
                        id: mk_event_id(&service_name, "recovery_suppressed_cooldown", &cycle_id),
                        kind: "recovery_suppressed_cooldown".to_string(),
                        severity: "info".to_string(),
                        target: sid.clone(),
                        message: reason,
                        correlation_id: Some(correlation_id.clone()),
                        source_plugin: Some("recovery_planner".to_string()),
                        status: Some("suppressed".to_string()),
                        details: Some(json!({
                            "service_name": service_name,
                            "cooldown_s": cooldown_s,
                            "last_attempt_at_utc": last_attempt_at,
                            "seconds_since_last_attempt": delta_s,
                            "attempts_in_window": attempts_in_window,
                            "attempt_window_s": attempt_window_s,
                            "max_attempts_per_window": max_attempts_per_window,
                            "counted_origins": ["recovery_planner", "desired_state_planner"]
                        })),
                        observed_at_utc: observed_at,
                    });

                    continue;
                }
            }

            if attempts_in_window >= max_attempts_per_window {
                let reason = format!(
                    "attempt window exceeded for '{}': {} attempts in {}s (max={})",
                    service_name, attempts_in_window, attempt_window_s, max_attempts_per_window
                );

                let action = ActionRecord {
                    id: mk_action_id(&correlation_id, &cycle_id),
                    correlation_id: correlation_id.clone(),
                    origin: "recovery_planner".to_string(),
                    kind: "noop".to_string(),
                    phase: "planned".to_string(),
                    status: "suppressed".to_string(),
                    service_id: sid.clone(),
                    service_name: service_name.clone(),
                    target_kind: "service".to_string(),
                    target_id: sid.clone(),
                    reason: Some("recovery attempt-window suppression".to_string()),
                    message: Some(reason.clone()),
                    dry_run,
                    suppressed: true,
                    exit_code: None,
                    duration_ms: None,
                    stdout: None,
                    stderr: None,
                    observed_at_utc: observed_at,
                };

                new_actions.push(action);

                new_events.push(Event {
                    id: mk_event_id(
                        &service_name,
                        "recovery_suppressed_attempt_window",
                        &cycle_id,
                    ),
                    kind: "recovery_suppressed_attempt_window".to_string(),
                    severity: "warning".to_string(),
                    target: sid.clone(),
                    message: reason,
                    correlation_id: Some(correlation_id.clone()),
                    source_plugin: Some("recovery_planner".to_string()),
                    status: Some("suppressed".to_string()),
                    details: Some(json!({
                        "service_name": service_name,
                        "cooldown_s": cooldown_s,
                        "last_attempt_at_utc": last_attempt_at,
                        "seconds_since_last_attempt": seconds_since_last_attempt,
                        "attempts_in_window": attempts_in_window,
                        "attempt_window_s": attempt_window_s,
                        "max_attempts_per_window": max_attempts_per_window,
                        "counted_origins": ["recovery_planner", "desired_state_planner"]
                    })),
                    observed_at_utc: observed_at,
                });

                continue;
            }

            let plan = plan_from_registry(snapshot, &service_name, registry_service);

            let action_status = if plan.suppressed {
                "suppressed".to_string()
            } else {
                "pending".to_string()
            };

            let suppression_reason_for_message = plan
                .suppression_reason
                .clone()
                .unwrap_or_else(|| "unknown reason".to_string());

            let suppression_reason_for_details = plan.suppression_reason.clone();

            let (event_kind, severity, message) = if plan.suppressed {
                (
                    "recovery_suppressed".to_string(),
                    "info".to_string(),
                    format!(
                        "Recovery plan suppressed for service '{}': {}",
                        plan.service_name, suppression_reason_for_message
                    ),
                )
            } else {
                (
                    "recovery_planned".to_string(),
                    "warning".to_string(),
                    format!(
                        "Recovery planned for service '{}': kind={} target_kind={} target_id={}",
                        plan.service_name, plan.action_kind, plan.target_kind, plan.target_id
                    ),
                )
            };

            let action = ActionRecord {
                id: mk_action_id(&correlation_id, &cycle_id),
                correlation_id: correlation_id.clone(),
                origin: "recovery_planner".to_string(),
                kind: plan.action_kind.clone(),
                phase: "planned".to_string(),
                status: action_status,
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
            };

            new_actions.push(action);

            new_events.push(Event {
                id: mk_event_id(&plan.service_name, &event_kind, &cycle_id),
                kind: event_kind,
                severity,
                target: plan.service_id.clone(),
                message,
                correlation_id: Some(correlation_id),
                source_plugin: Some("recovery_planner".to_string()),
                status: Some(if plan.suppressed {
                    "suppressed".to_string()
                } else {
                    "planned".to_string()
                }),
                details: Some(json!({
                    "service_name": plan.service_name,
                    "service_status": plan.service_status,
                    "action_kind": plan.action_kind,
                    "target_kind": plan.target_kind,
                    "target_id": plan.target_id,
                    "reason": plan.reason,
                    "suppressed": plan.suppressed,
                    "suppression_reason": suppression_reason_for_details,
                    "dry_run": dry_run,
                    "last_attempt_at_utc": last_attempt_at,
                    "seconds_since_last_attempt": seconds_since_last_attempt,
                    "attempts_in_window": attempts_in_window,
                    "attempt_window_s": attempt_window_s,
                    "max_attempts_per_window": max_attempts_per_window,
                    "counted_origins": ["recovery_planner", "desired_state_planner"]
                })),
                observed_at_utc: observed_at,
            });
        }

        snapshot.actions.extend(new_actions);
        snapshot.events.extend(new_events);

        Ok(())
    }
}
