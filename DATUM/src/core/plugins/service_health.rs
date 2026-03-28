use anyhow::Result;
use serde_json::{json, Value as JsonValue};

use crate::core::ids;
use crate::core::model::{Entity, Event, Snapshot};
use crate::core::plugin::Plugin;
use crate::core::plugins::checks::http::run_http_health;
use crate::core::plugins::checks::process::run_process_alive;
use crate::core::plugins::checks::tcp::run_tcp_port;

pub struct ServiceHealthPlugin;

impl ServiceHealthPlugin {
    pub fn new() -> Self {
        Self
    }
}

fn mk_event_id(cycle_id: &str, service_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let s = ids::sanitize_id_component(service_id);
    let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
}

fn result_is_available(result: &JsonValue) -> bool {
    matches!(
        result.get("status").and_then(serde_json::Value::as_str),
        Some("available") | Some("ok")
    )
}

fn find_connection_entity<'a>(snapshot: &'a Snapshot, wanted: &str) -> Option<&'a Entity> {
    let wanted_lc = wanted.trim().to_lowercase();

    snapshot.entities.iter().find(|e| {
        if e.kind != "connection" {
            return false;
        }

        let candidate = e
            .attrs
            .get("target")
            .and_then(|v| v.as_str())
            .or_else(|| e.attrs.get("to").and_then(|v| v.as_str()));

        match candidate {
            Some(c) => c.trim().to_lowercase() == wanted_lc,
            None => false,
        }
    })
}

fn connection_is_available(conn: &Entity) -> bool {
    conn.attrs
        .get("available")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn find_service_entity_by_name<'a>(snapshot: &'a Snapshot, wanted: &str) -> Option<&'a Entity> {
    let wanted_lc = wanted.trim().to_lowercase();

    snapshot.entities.iter().find(|e| {
        if e.kind != "service" {
            return false;
        }

        let candidate = e.attrs.get("name").and_then(|v| v.as_str());
        match candidate {
            Some(name) => name.trim().to_lowercase() == wanted_lc,
            None => false,
        }
    })
}

impl Plugin for ServiceHealthPlugin {
    fn name(&self) -> &'static str {
        "service_health"
    }

    fn apply(&self, snapshot: &mut Snapshot, _cfg: &crate::config::Config) -> Result<()> {
        let observed_at = snapshot.meta.observed_at_utc;
        let observed_at_str = observed_at.to_rfc3339();
        let cycle_id = snapshot.meta.cycle_id.clone();

        for idx in 0..snapshot.entities.len() {
            if snapshot.entities[idx].kind != "service" {
                continue;
            }

            let service_id = snapshot.entities[idx].id.clone();
            let service_name = snapshot.entities[idx]
                .attrs
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&service_id)
                .to_string();

            let service_attrs_clone = snapshot.entities[idx].attrs.clone();

            let bound = service_attrs_clone
                .get("binding")
                .and_then(|b| b.get("bound"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            // Sem binding resolvido => indisponível
            if !bound {
                snapshot.entities[idx].attrs["status"] = json!("unavailable");
                snapshot.entities[idx].attrs["checks_result"] = json!([]);
                continue;
            }

            let checks_declared = service_attrs_clone
                .get("checks_declared")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            // Sem checks => unknown
            if checks_declared.is_empty() {
                snapshot.entities[idx].attrs["status"] = json!("unknown");
                snapshot.entities[idx].attrs["checks_result"] = json!([]);
                continue;
            }

            let mut results: Vec<JsonValue> = Vec::new();
            let mut all_checks_available = true;

            for chk in checks_declared {
                let kind = chk.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                let params = chk.get("params").cloned().unwrap_or(json!({}));

                let result = match kind {
                    "http_health" => {
                        let url = params
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let timeout_ms = params
                            .get("timeout_ms")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1200);

                        run_http_health(url, timeout_ms, &observed_at_str)
                    }

                    "process_alive" => {
                        run_process_alive(snapshot, &service_attrs_clone, &observed_at_str)
                    }

                    "tcp_port" => {
                        let host = params
                            .get("host")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");

                        let port = params
                            .get("port")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u16;

                        let timeout_ms = params
                            .get("timeout_ms")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1000);

                        run_tcp_port(host, port, timeout_ms, &observed_at_str)
                    }

                    _ => json!({
                        "kind": kind,
                        "status": "unavailable",
                        "error": "unsupported_check_kind",
                        "checked_at": observed_at_str
                    }),
                };

                let ok = result_is_available(&result);

                if !ok {
                    all_checks_available = false;

                    snapshot.events.push(Event {
                        id: mk_event_id(&cycle_id, &service_id, &format!("check_failed_{}", kind)),
                        kind: "service_check_failed".to_string(),
                        severity: "warning".to_string(),
                        target: service_id.clone(),
                        message: format!("Service check '{}' failed for '{}'", kind, service_name),
                        observed_at_utc: observed_at,
                    });
                }

                results.push(result);
            }

            snapshot.entities[idx].attrs["checks_result"] = json!(results);

            // status base: resultado dos checks próprios
            let mut final_status = if all_checks_available {
                "available"
            } else {
                "unavailable"
            };

            // só propaga dependência se o próprio serviço já está disponível
            if final_status == "available" {
                let depends_on_declared = service_attrs_clone
                    .get("depends_on_declared")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                for dep in depends_on_declared {
                    let dep_kind = dep.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    let target = dep.get("target").and_then(|v| v.as_str()).unwrap_or("");

                    if target.trim().is_empty() {
                        continue;
                    }

                    match dep_kind {
                        "connection" => {
                            if let Some(conn) = find_connection_entity(snapshot, target) {
                                if !connection_is_available(conn) {
                                    final_status = "degraded";

                                    snapshot.events.push(Event {
                                        id: mk_event_id(
                                            &cycle_id,
                                            &service_id,
                                            &format!("dependency_connection_degraded_{}", target),
                                        ),
                                        kind: "service_dependency_degraded".to_string(),
                                        severity: "warning".to_string(),
                                        target: service_id.clone(),
                                        message: format!(
                                            "Service '{}' is degraded because dependency connection '{}' is unavailable",
                                            service_name, target
                                        ),
                                        observed_at_utc: observed_at,
                                    });

                                    break;
                                }
                            }
                        }

                        "service" => {
                            if let Some(dep_service) = find_service_entity_by_name(snapshot, target) {
                                let dep_status = dep_service
                                    .attrs
                                    .get("status")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown");

                                if matches!(dep_status, "unavailable" | "degraded") {
                                    final_status = "degraded";

                                    snapshot.events.push(Event {
                                        id: mk_event_id(
                                            &cycle_id,
                                            &service_id,
                                            &format!("dependency_service_degraded_{}", target),
                                        ),
                                        kind: "service_dependency_degraded".to_string(),
                                        severity: "warning".to_string(),
                                        target: service_id.clone(),
                                        message: format!(
                                            "Service '{}' is degraded because dependent service '{}' is {}",
                                            service_name, target, dep_status
                                        ),
                                        observed_at_utc: observed_at,
                                    });

                                    break;
                                }
                            }
                        }

                        _ => {}
                    }
                }
            }

            snapshot.entities[idx].attrs["status"] = json!(final_status);
        }

        Ok(())
    }
}