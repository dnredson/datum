use anyhow::Result;
use serde_json::json;

use crate::config::{Config, ServiceRegistryService};
use crate::core::ids;
use crate::core::model::{ActionRecord, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct DesiredStatePlannerPlugin;

impl DesiredStatePlannerPlugin {
    pub fn new() -> Self {
        Self
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn mk_correlation_id(service_name: &str, cycle_id: &str) -> String {
    let s = ids::sanitize_id_component(service_name);
    let c = ids::sanitize_id_component(cycle_id);
    format!("desired_state:{}:{}", s, c)
}

fn mk_action_id(cycle_id: &str, correlation_id: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let x = ids::sanitize_id_component(correlation_id);
    format!("action:{}:planned:{}", x, c)
}

fn mk_event_id(cycle_id: &str, service_name: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let s = ids::sanitize_id_component(service_name);
    let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
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

fn service_status(snapshot: &Snapshot, name: &str) -> Option<String> {
    service_entity(snapshot, name)?
        .attrs
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn service_runtime(snapshot: &Snapshot, name: &str) -> Option<String> {
    service_entity(snapshot, name)?
        .attrs
        .get("runtime")
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

fn planned_action_exists(snapshot: &Snapshot, service_name: &str) -> bool {
    let wanted = norm(service_name);
    snapshot
        .actions
        .iter()
        .any(|a| a.phase == "planned" && norm(&a.service_name) == wanted)
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
) -> Option<(String, String, String, bool, Option<String>)> {
    let runtime = service_runtime(snapshot, service_name).unwrap_or_else(|| svc.runtime.clone());
    let recovery = svc.recovery.as_ref()?;
    let policy = recovery.policy.trim();

    if policy.is_empty() || !policy_matches_runtime(&runtime, policy) {
        return Some((
            "noop".to_string(),
            "service".to_string(),
            service_id(snapshot, service_name),
            true,
            Some(format!(
                "recovery policy '{}' is incompatible with runtime '{}'",
                policy, runtime
            )),
        ));
    }

    match policy {
        "restart_command" => Some((
            "restart_command".to_string(),
            "service".to_string(),
            service_id(snapshot, service_name),
            false,
            None,
        )),
        "docker_restart" => {
            if service_has_binding(snapshot, service_name) {
                let tid = service_entity(snapshot, service_name)?
                    .attrs
                    .get("bindings")
                    .and_then(|v| v.as_array())?
                    .iter()
                    .find_map(|b| {
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
                    });

                tid.map(|x| {
                    (
                        "restart_container".to_string(),
                        "container".to_string(),
                        x,
                        false,
                        None,
                    )
                })
            } else if let Some(name) = declared_container_name(snapshot, service_name) {
                Some((
                    "start_container".to_string(),
                    "container_name".to_string(),
                    name,
                    false,
                    None,
                ))
            } else {
                Some((
                    "escalate".to_string(),
                    "service".to_string(),
                    service_id(snapshot, service_name),
                    true,
                    Some("container service has no recoverable binding".to_string()),
                ))
            }
        }
        "docker_start" => {
            if let Some(name) = declared_container_name(snapshot, service_name) {
                Some((
                    "start_container".to_string(),
                    "container_name".to_string(),
                    name,
                    false,
                    None,
                ))
            } else {
                Some((
                    "escalate".to_string(),
                    "service".to_string(),
                    service_id(snapshot, service_name),
                    true,
                    Some("container service has no declared container_name binding".to_string()),
                ))
            }
        }
        _ => Some((
            "escalate".to_string(),
            "service".to_string(),
            service_id(snapshot, service_name),
            true,
            Some(format!(
                "unsupported desired_state policy '{}'",
                policy
            )),
        )),
    }
}

impl Plugin for DesiredStatePlannerPlugin {
    fn name(&self) -> &'static str {
        "desired_state_planner"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &Config) -> Result<()> {
        let dcfg = match cfg.desired_state.as_ref() {
            Some(d) if d.enabled => d,
            _ => return Ok(()),
        };

        let dry_run = cfg
            .auto_recovery
            .as_ref()
            .and_then(|r| r.dry_run)
            .unwrap_or(true);

        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();

        let mut new_actions = Vec::new();
        let mut new_events = Vec::new();

        for ds in &dcfg.services {
            if norm(&ds.desired) != "running" {
                continue;
            }

            if planned_action_exists(snapshot, &ds.name) {
                continue;
            }

            let status = service_status(snapshot, &ds.name).unwrap_or_else(|| "unknown".to_string());
            if status == "available" {
                continue;
            }

            let service_target_id = service_id(snapshot, &ds.name);

            let Some(registry_service) = cfg.find_registry_service(&ds.name) else {
                new_events.push(Event {
                    id: mk_event_id(&cycle_id, &ds.name, "desired_state_missing_registry"),
                    kind: "desired_state_missing_registry".to_string(),
                    severity: "warning".to_string(),
                    target: service_target_id,
                    message: format!(
                        "desired_state for service '{}' has no matching service_registry entry",
                        ds.name
                    ),
                    correlation_id: None,
                    source_plugin: Some("desired_state_planner".to_string()),
                    status: Some("open".to_string()),
                    details: Some(json!({
                        "service_name": ds.name,
                        "desired": ds.desired,
                        "reason": "missing service_registry entry"
                    })),
                    observed_at_utc: observed_at,
                });
                continue;
            };

            let Some((kind, target_kind, target_id, suppressed, suppression_reason)) =
                plan_from_registry(snapshot, &ds.name, registry_service)
            else {
                continue;
            };

            let correlation_id = mk_correlation_id(&ds.name, &cycle_id);

            let action_reason =
                Some(format!("desired_state=running but observed status is {}", status));
            let action_status = if suppressed { "suppressed" } else { "pending" }.to_string();
            let action_message = suppression_reason.clone();

            let action = ActionRecord {
                id: mk_action_id(&cycle_id, &correlation_id),
                correlation_id: correlation_id.clone(),
                origin: "desired_state_planner".to_string(),
                kind: kind.clone(),
                phase: "planned".to_string(),
                status: action_status,
                service_id: service_target_id.clone(),
                service_name: ds.name.clone(),
                target_kind: target_kind.clone(),
                target_id: target_id.clone(),
                reason: action_reason,
                message: action_message.clone(),
                dry_run,
                suppressed,
                exit_code: None,
                duration_ms: None,
                stdout: None,
                stderr: None,
                observed_at_utc: observed_at,
            };

            new_actions.push(action);

            let event_kind = if suppressed {
                "desired_state_action_suppressed"
            } else {
                "desired_state_action_planned"
            };

            let severity = if suppressed { "info" } else { "warning" };

            let msg = if let Some(reason) = suppression_reason.clone() {
                format!(
                    "desired_state planned action for service '{}': {}",
                    ds.name, reason
                )
            } else {
                format!(
                    "desired_state planned action for service '{}': kind={} desired=running observed={}",
                    ds.name, kind, status
                )
            };

            new_events.push(Event {
                id: mk_event_id(&cycle_id, &ds.name, event_kind),
                kind: event_kind.to_string(),
                severity: severity.to_string(),
                target: service_target_id,
                message: msg,
                correlation_id: Some(correlation_id),
                source_plugin: Some("desired_state_planner".to_string()),
                status: Some(if suppressed {
                    "suppressed".to_string()
                } else {
                    "planned".to_string()
                }),
                details: Some(json!({
                    "service_name": ds.name,
                    "desired": ds.desired,
                    "observed_status": status,
                    "action_kind": kind,
                    "target_kind": target_kind,
                    "target_id": target_id,
                    "dry_run": dry_run,
                    "suppression_reason": action_message
                })),
                observed_at_utc: observed_at,
            });
        }

        snapshot.actions.extend(new_actions);
        snapshot.events.extend(new_events);
        Ok(())
    }
}