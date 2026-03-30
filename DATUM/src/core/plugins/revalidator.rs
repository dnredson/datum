use std::collections::HashMap;

use anyhow::Result;
use serde_json::{json, Value as JsonValue};

use crate::config::Config;
use crate::core::ids;
use crate::core::model::{ActionRecord, Entity, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct RevalidatorPlugin;

impl RevalidatorPlugin {
    pub fn new() -> Self {
        Self
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn is_remediation_origin(origin: &str) -> bool {
    origin == "recovery_planner" || origin == "desired_state_planner"
}

fn is_terminal_action(action: &ActionRecord) -> bool {
    if !is_remediation_origin(&action.origin) {
        return false;
    }

    matches!(action.phase.as_str(), "planned" | "executing" | "finished")
        && matches!(
            action.status.as_str(),
            "succeeded" | "failed" | "error" | "resolved" | "skipped" | "suppressed"
        )
}

fn is_success_status(status: &str) -> bool {
    matches!(status, "succeeded" | "resolved")
}

fn is_failure_status(status: &str) -> bool {
    matches!(status, "failed" | "error")
}

fn is_skipped_status(status: &str) -> bool {
    matches!(status, "skipped" | "suppressed")
}

fn terminal_actions_by_correlation(snapshot: &Snapshot) -> Vec<ActionRecord> {
    let mut latest: HashMap<String, ActionRecord> = HashMap::new();

    for action in &snapshot.actions {
        if !is_terminal_action(action) {
            continue;
        }

        let entry = latest
            .entry(action.correlation_id.clone())
            .or_insert_with(|| action.clone());

        if action.observed_at_utc > entry.observed_at_utc {
            *entry = action.clone();
        }
    }

    latest.into_values().collect()
}

fn service_entity<'a>(snapshot: &'a Snapshot, service_name: &str) -> Option<&'a Entity> {
    let wanted = norm(service_name);

    snapshot.entities.iter().find(|e| {
        e.kind == "service"
            && e.attrs
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| norm(s) == wanted)
                .unwrap_or(false)
    })
}

fn service_id(snapshot: &Snapshot, service_name: &str) -> String {
    service_entity(snapshot, service_name)
        .map(|e| e.id.clone())
        .unwrap_or_else(|| ids::service_id(service_name))
}

fn service_status(snapshot: &Snapshot, service_name: &str) -> String {
    service_entity(snapshot, service_name)
        .and_then(|e| e.attrs.get("status"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn service_checks_result(snapshot: &Snapshot, service_name: &str) -> JsonValue {
    service_entity(snapshot, service_name)
        .and_then(|e| e.attrs.get("checks_result"))
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn has_event_kind_for_correlation(snapshot: &Snapshot, correlation_id: &str, kind: &str) -> bool {
    snapshot.events.iter().any(|e| {
        e.kind == kind
            && e.correlation_id
                .as_ref()
                .map(|c| c == correlation_id)
                .unwrap_or(false)
    })
}

fn has_any_local_incident_closure(snapshot: &Snapshot, correlation_id: &str) -> bool {
    let closure_kinds = [
        "incident_resolved_local",
        "incident_failed_local",
        "incident_suppressed_local",
    ];

    snapshot.events.iter().any(|e| {
        closure_kinds.contains(&e.kind.as_str())
            && e.correlation_id
                .as_ref()
                .map(|c| c == correlation_id)
                .unwrap_or(false)
    })
}

fn mk_event_id(correlation_id: &str, suffix: &str, cycle_id: &str) -> String {
    let c = ids::sanitize_id_component(correlation_id);
    let s = ids::sanitize_id_component(suffix);
    let cy = ids::sanitize_id_component(cycle_id);
    format!("event:{}:{}:{}", c, s, cy)
}

fn mk_verification_entity_id(correlation_id: &str, cycle_id: &str) -> String {
    let c = ids::sanitize_id_component(correlation_id);
    let cy = ids::sanitize_id_component(cycle_id);
    format!("recovery_verification:{}:{}", c, cy)
}

fn verification_entity(
    correlation_id: &str,
    cycle_id: &str,
    action: &ActionRecord,
    validation_status: &str,
    verification_status: &str,
    checks_result: JsonValue,
) -> Entity {
    Entity {
        id: mk_verification_entity_id(correlation_id, cycle_id),
        kind: "recovery_verification".to_string(),
        attrs: json!({
            "correlation_id": correlation_id,
            "origin": action.origin,
            "service_id": action.service_id,
            "service_name": action.service_name,
            "action_kind": action.kind,
            "action_phase": action.phase,
            "action_status": action.status,
            "validation_status": validation_status,
            "verification_status": verification_status,
            "checks_result": checks_result
        }),
        observed_at_utc: action.observed_at_utc,
        ttl_s: 60,
    }
}

impl Plugin for RevalidatorPlugin {
    fn name(&self) -> &'static str {
        "revalidator"
    }

    fn apply(&self, snapshot: &mut Snapshot, _cfg: &Config) -> Result<()> {
        let cycle_id = snapshot.meta.cycle_id.clone();
        let observed_at = snapshot.meta.observed_at_utc;

        let terminal_actions = terminal_actions_by_correlation(snapshot);

        let mut new_entities = Vec::new();
        let mut new_events = Vec::new();

        for action in terminal_actions {
            let correlation_id = action.correlation_id.clone();

            if has_any_local_incident_closure(snapshot, &correlation_id) {
                continue;
            }

            let sid = if action.service_id.is_empty() {
                service_id(snapshot, &action.service_name)
            } else {
                action.service_id.clone()
            };

            let current_status = service_status(snapshot, &action.service_name);
            let checks_result = service_checks_result(snapshot, &action.service_name);

            if is_skipped_status(&action.status) || action.suppressed {
                let verification_status = "suppressed";
                let validation_status = current_status.clone();

                new_entities.push(verification_entity(
                    &correlation_id,
                    &cycle_id,
                    &action,
                    &validation_status,
                    verification_status,
                    checks_result.clone(),
                ));

                if !has_event_kind_for_correlation(
                    snapshot,
                    &correlation_id,
                    "incident_suppressed_local",
                ) {
                    new_events.push(Event {
                        id: mk_event_id(&correlation_id, "incident_suppressed_local", &cycle_id),
                        kind: "incident_suppressed_local".to_string(),
                        severity: "info".to_string(),
                        target: sid,
                        message: format!(
                            "Local incident for service '{}' was suppressed",
                            action.service_name
                        ),
                        correlation_id: Some(correlation_id),
                        source_plugin: Some("revalidator".to_string()),
                        status: Some("suppressed".to_string()),
                        details: Some(json!({
                            "service_name": action.service_name,
                            "service_id": action.service_id,
                            "origin": action.origin,
                            "action_kind": action.kind,
                            "action_phase": action.phase,
                            "action_status": action.status,
                            "validation_status": validation_status,
                            "verification_status": verification_status,
                            "checks_result": checks_result
                        })),
                        observed_at_utc: observed_at,
                    });
                }

                continue;
            }

            if is_failure_status(&action.status) {
                let verification_status = "failed";
                let validation_status = current_status.clone();

                new_entities.push(verification_entity(
                    &correlation_id,
                    &cycle_id,
                    &action,
                    &validation_status,
                    verification_status,
                    checks_result.clone(),
                ));

                if !has_event_kind_for_correlation(snapshot, &correlation_id, "revalidation_failed")
                {
                    new_events.push(Event {
                        id: mk_event_id(&correlation_id, "revalidation_failed", &cycle_id),
                        kind: "revalidation_failed".to_string(),
                        severity: "critical".to_string(),
                        target: sid.clone(),
                        message: format!(
                            "Revalidation failed for service '{}'",
                            action.service_name
                        ),
                        correlation_id: Some(correlation_id.clone()),
                        source_plugin: Some("revalidator".to_string()),
                        status: Some("failed".to_string()),
                        details: Some(json!({
                            "service_name": action.service_name,
                            "service_id": action.service_id,
                            "origin": action.origin,
                            "action_kind": action.kind,
                            "action_phase": action.phase,
                            "action_status": action.status,
                            "validation_status": validation_status,
                            "verification_status": verification_status,
                            "checks_result": checks_result
                        })),
                        observed_at_utc: observed_at,
                    });
                }

                if !has_event_kind_for_correlation(
                    snapshot,
                    &correlation_id,
                    "incident_failed_local",
                ) {
                    new_events.push(Event {
                        id: mk_event_id(&correlation_id, "incident_failed_local", &cycle_id),
                        kind: "incident_failed_local".to_string(),
                        severity: "critical".to_string(),
                        target: sid,
                        message: format!(
                            "Local incident for service '{}' ended in failure",
                            action.service_name
                        ),
                        correlation_id: Some(correlation_id),
                        source_plugin: Some("revalidator".to_string()),
                        status: Some("failed".to_string()),
                        details: Some(json!({
                            "service_name": action.service_name,
                            "service_id": action.service_id,
                            "origin": action.origin,
                            "action_kind": action.kind,
                            "action_phase": action.phase,
                            "action_status": action.status,
                            "validation_status": validation_status,
                            "verification_status": verification_status,
                            "checks_result": checks_result
                        })),
                        observed_at_utc: observed_at,
                    });
                }

                continue;
            }

            if is_success_status(&action.status) {
                if current_status == "available" {
                    let verification_status = "confirmed";
                    let validation_status = "available";

                    new_entities.push(verification_entity(
                        &correlation_id,
                        &cycle_id,
                        &action,
                        validation_status,
                        verification_status,
                        checks_result.clone(),
                    ));

                    if !has_event_kind_for_correlation(snapshot, &correlation_id, "recovery_confirmed")
                    {
                        new_events.push(Event {
                            id: mk_event_id(&correlation_id, "recovery_confirmed", &cycle_id),
                            kind: "recovery_confirmed".to_string(),
                            severity: "warning".to_string(),
                            target: sid.clone(),
                            message: format!(
                                "Recovery confirmed for service '{}'",
                                action.service_name
                            ),
                            correlation_id: Some(correlation_id.clone()),
                            source_plugin: Some("revalidator".to_string()),
                            status: Some("resolved".to_string()),
                            details: Some(json!({
                                "service_name": action.service_name,
                                "service_id": action.service_id,
                                "origin": action.origin,
                                "action_kind": action.kind,
                                "action_phase": action.phase,
                                "action_status": action.status,
                                "validation_status": validation_status,
                                "verification_status": verification_status,
                                "checks_result": checks_result
                            })),
                            observed_at_utc: observed_at,
                        });
                    }

                    if !has_event_kind_for_correlation(
                        snapshot,
                        &correlation_id,
                        "incident_resolved_local",
                    ) {
                        new_events.push(Event {
                            id: mk_event_id(&correlation_id, "incident_resolved_local", &cycle_id),
                            kind: "incident_resolved_local".to_string(),
                            severity: "info".to_string(),
                            target: sid,
                            message: format!(
                                "Local incident for service '{}' was resolved",
                                action.service_name
                            ),
                            correlation_id: Some(correlation_id),
                            source_plugin: Some("revalidator".to_string()),
                            status: Some("resolved".to_string()),
                            details: Some(json!({
                                "service_name": action.service_name,
                                "service_id": action.service_id,
                                "origin": action.origin,
                                "action_kind": action.kind,
                                "action_phase": action.phase,
                                "action_status": action.status,
                                "validation_status": validation_status,
                                "verification_status": verification_status,
                                "checks_result": checks_result
                            })),
                            observed_at_utc: observed_at,
                        });
                    }
                } else {
                    let verification_status = "pending_observation";
                    let validation_status = current_status.clone();

                    new_entities.push(verification_entity(
                        &correlation_id,
                        &cycle_id,
                        &action,
                        &validation_status,
                        verification_status,
                        checks_result.clone(),
                    ));

                    if !has_event_kind_for_correlation(
                        snapshot,
                        &correlation_id,
                        "recovery_pending_observation",
                    ) {
                        new_events.push(Event {
                            id: mk_event_id(
                                &correlation_id,
                                "recovery_pending_observation",
                                &cycle_id,
                            ),
                            kind: "recovery_pending_observation".to_string(),
                            severity: "info".to_string(),
                            target: sid,
                            message: format!(
                                "Recovery for service '{}' executed, but confirmation should be observed in the next cycle",
                                action.service_name
                            ),
                            correlation_id: Some(correlation_id),
                            source_plugin: Some("revalidator".to_string()),
                            status: Some("pending_observation".to_string()),
                            details: Some(json!({
                                "service_name": action.service_name,
                                "service_id": action.service_id,
                                "origin": action.origin,
                                "action_kind": action.kind,
                                "action_phase": action.phase,
                                "action_status": action.status,
                                "validation_status": validation_status,
                                "verification_status": verification_status,
                                "checks_result": checks_result
                            })),
                            observed_at_utc: observed_at,
                        });
                    }
                }
            }
        }

        snapshot.entities.extend(new_entities);
        snapshot.events.extend(new_events);

        Ok(())
    }
}