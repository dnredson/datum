use anyhow::Result;
use serde_json::{json, Value as JsonValue};

use crate::core::ids;
use crate::core::model::{Edge, Entity, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct ServiceBuilderPlugin;

impl ServiceBuilderPlugin {
    pub fn new() -> Self {
        Self
    }
}

/// Normaliza nomes para matching cross-platform.
/// - trim + lowercase
/// - remove aspas
/// - remove caminho ("/usr/bin/sshd" -> "sshd", "C:\x\datum.exe" -> "datum.exe")
/// - remove extensão .exe (pra facilitar "datum" == "datum.exe")
fn norm_key(raw: &str) -> String {
    let mut s = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_lowercase();

    if let Some(pos) = s.rfind('/') {
        s = s[pos + 1..].to_string();
    }
    if let Some(pos) = s.rfind('\\') {
        s = s[pos + 1..].to_string();
    }

    if let Some(stripped) = s.strip_suffix(".exe") {
        s = stripped.to_string();
    }

    s
}

fn container_name_candidates(attrs: &JsonValue) -> Vec<String> {
    let mut out = Vec::new();

    if let Some(s) = attrs.get("name").and_then(|v| v.as_str()) {
        for part in s.split(',') {
            let k = norm_key(part);
            if !k.is_empty() {
                out.push(k);
            }
        }
    }

    if let Some(s) = attrs.get("container_name").and_then(|v| v.as_str()) {
        let k = norm_key(s);
        if !k.is_empty() {
            out.push(k);
        }
    }

    if let Some(arr) = attrs.get("names").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                let k = norm_key(s);
                if !k.is_empty() {
                    out.push(k);
                }
            }
        }
    }

    out
}

fn process_name_candidates(attrs: &JsonValue) -> Vec<String> {
    let mut out = Vec::new();
    let keys = ["name", "image_name", "comm", "exe", "binary", "process_name"];

    for k in keys {
        if let Some(s) = attrs.get(k).and_then(|v| v.as_str()) {
            let n = norm_key(s);
            if !n.is_empty() {
                out.push(n);
            }
        }
    }

    out
}

fn find_container_by_name(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    for e in snapshot.entities.iter() {
        if e.kind != "container" {
            continue;
        }

        let candidates = container_name_candidates(&e.attrs);
        if candidates.iter().any(|c| c == &wanted) {
            return Some(e.id.clone());
        }
    }

    None
}

fn find_process_group_by_name(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    for e in snapshot.entities.iter() {
        if e.kind != "process" {
            continue;
        }

        let candidates = process_name_candidates(&e.attrs);
        if candidates.iter().any(|c| c == &wanted) {
            return Some(e.id.clone());
        }
    }

    None
}

fn find_connection_by_target(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    for e in snapshot.entities.iter() {
        if e.kind != "connection" {
            continue;
        }

        let candidate = e
            .attrs
            .get("target")
            .and_then(|v| v.as_str())
            .or_else(|| e.attrs.get("to").and_then(|v| v.as_str()));

        if let Some(target) = candidate {
            if norm_key(target) == wanted {
                return Some(e.id.clone());
            }
        }
    }

    None
}

fn find_service_by_name(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    for e in snapshot.entities.iter() {
        if e.kind != "service" {
            continue;
        }

        if let Some(name) = e.attrs.get("name").and_then(|v| v.as_str()) {
            if norm_key(name) == wanted {
                return Some(e.id.clone());
            }
        }
    }

    None
}

fn mk_event_id(cycle_id: &str, service_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let s = ids::sanitize_id_component(service_id);
    let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
}

fn check_spec_to_json(check: &crate::config::ServiceCheckSpec) -> JsonValue {
    let mut params_map = serde_json::Map::new();

    for (k, v) in check.params.iter() {
        let json_value = match serde_json::to_value(v) {
            Ok(v) => v,
            Err(_) => JsonValue::Null,
        };
        params_map.insert(k.clone(), json_value);
    }

    json!({
        "kind": check.kind,
        "params": params_map
    })
}

fn dependency_spec_to_json(dep: &crate::config::ServiceDependencySpec) -> JsonValue {
    json!({
        "kind": dep.kind,
        "target": dep.target
    })
}

impl Plugin for ServiceBuilderPlugin {
    fn name(&self) -> &'static str {
        "service_builder"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        let scfg = match cfg.services.as_ref() {
            Some(s) if s.enabled => s,
            _ => return Ok(()),
        };

        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();

        // =========================================================
        // PASS 1: cria todas as entidades service primeiro
        // =========================================================
        for item in scfg.items.iter() {
            let service_eid = ids::service_id(&item.name);

            // evita duplicar se por algum motivo já existir
            let already_exists = snapshot
                .entities
                .iter()
                .any(|e| e.kind == "service" && e.id == service_eid);

            if already_exists {
                continue;
            }

            let checks_declared: Vec<JsonValue> =
                item.checks.iter().map(check_spec_to_json).collect();

            let depends_on_declared: Vec<JsonValue> =
                item.depends_on.iter().map(dependency_spec_to_json).collect();

            let ent = Entity {
                id: service_eid,
                kind: "service".to_string(),
                attrs: json!({
                    "name": item.name,
                    "status": "unknown",
                    "desired_state": "running",
                    "binding": {
                        "bound": false
                    },
                    "bindings": [],
                    "checks_declared": checks_declared,
                    "depends_on_declared": depends_on_declared
                }),
                observed_at_utc: observed_at,
                ttl_s: 60,
            };

            snapshot.entities.push(ent);
        }

        // =========================================================
        // PASS 2: resolve implemented_by e depends_on
        // =========================================================
        for item in scfg.items.iter() {
            let service_eid = ids::service_id(&item.name);

            let mut bindings_json: Vec<JsonValue> = Vec::new();
            let mut edges_to_push: Vec<Edge> = Vec::new();

            // implemented_by
            for impl_ in item.implemented_by.iter() {
                if let Some(ref cname) = impl_.container_name {
                    if let Some(cid) = find_container_by_name(snapshot, cname) {
                        bindings_json.push(json!({
                            "bound": true,
                            "target_id": cid,
                            "target_kind": "container",
                            "via": "container_name",
                            "wanted": cname
                        }));

                        let to_id = bindings_json
                            .last()
                            .and_then(|b| b.get("target_id"))
                            .and_then(|v| v.as_str())
                            .unwrap()
                            .to_string();

                        edges_to_push.push(Edge {
                            from: service_eid.clone(),
                            to: to_id,
                            rel: "implemented_by".to_string(),
                            attrs: json!({ "via": "container_name" }),
                            observed_at_utc: observed_at,
                        });
                        continue;
                    } else {
                        bindings_json.push(json!({
                            "bound": false,
                            "target_id": null,
                            "target_kind": "container",
                            "via": "container_name",
                            "wanted": cname
                        }));
                        continue;
                    }
                }

                if let Some(ref pname) = impl_.process_name {
                    if let Some(pid) = find_process_group_by_name(snapshot, pname) {
                        bindings_json.push(json!({
                            "bound": true,
                            "target_id": pid,
                            "target_kind": "process",
                            "via": "process_name",
                            "wanted": pname
                        }));

                        let to_id = bindings_json
                            .last()
                            .and_then(|b| b.get("target_id"))
                            .and_then(|v| v.as_str())
                            .unwrap()
                            .to_string();

                        edges_to_push.push(Edge {
                            from: service_eid.clone(),
                            to: to_id,
                            rel: "implemented_by".to_string(),
                            attrs: json!({ "via": "process_name" }),
                            observed_at_utc: observed_at,
                        });
                    } else {
                        bindings_json.push(json!({
                            "bound": false,
                            "target_id": null,
                            "target_kind": "process",
                            "via": "process_name",
                            "wanted": pname
                        }));
                    }
                }
            }

            let bound_any = bindings_json
                .iter()
                .any(|b| b.get("bound").and_then(|v| v.as_bool()) == Some(true));

            // atualiza a entidade service já criada no pass 1
            if let Some(service_ent) = snapshot
                .entities
                .iter_mut()
                .find(|e| e.kind == "service" && e.id == service_eid)
            {
                service_ent.attrs["binding"] = json!({
                    "bound": bound_any
                });
                service_ent.attrs["bindings"] = json!(bindings_json);
            }

            for e in edges_to_push {
                snapshot.edges.push(e);
            }

            if !bound_any {
                snapshot.events.push(Event {
                    id: mk_event_id(&cycle_id, &service_eid, "service_unbound"),
                    kind: "service_unbound".to_string(),
                    severity: "info".to_string(),
                    target: service_eid.clone(),
                    message: format!(
                        "Service '{}' has no resolved bindings (check implemented_by in config)",
                        item.name
                    ),
                    observed_at_utc: observed_at,
                });
            }

            // depends_on
            for dep in item.depends_on.iter() {
                let target = match dep.target.as_ref() {
                    Some(t) if !t.trim().is_empty() => t,
                    _ => continue,
                };

                match dep.kind.as_str() {
                    "connection" => {
                        if let Some(conn_id) = find_connection_by_target(snapshot, target) {
                            snapshot.edges.push(Edge {
                                from: service_eid.clone(),
                                to: conn_id,
                                rel: "depends_on".to_string(),
                                attrs: json!({
                                    "kind": "connection",
                                    "target": target
                                }),
                                observed_at_utc: observed_at,
                            });
                        } else {
                            snapshot.events.push(Event {
                                id: mk_event_id(
                                    &cycle_id,
                                    &service_eid,
                                    &format!("dependency_connection_unresolved_{}", target),
                                ),
                                kind: "service_dependency_unresolved".to_string(),
                                severity: "info".to_string(),
                                target: service_eid.clone(),
                                message: format!(
                                    "Service '{}' declares dependency on connection '{}' but no matching connection entity was found",
                                    item.name, target
                                ),
                                observed_at_utc: observed_at,
                            });
                        }
                    }

                    "service" => {
                        if let Some(dep_service_id) = find_service_by_name(snapshot, target) {
                            snapshot.edges.push(Edge {
                                from: service_eid.clone(),
                                to: dep_service_id,
                                rel: "depends_on".to_string(),
                                attrs: json!({
                                    "kind": "service",
                                    "target": target
                                }),
                                observed_at_utc: observed_at,
                            });
                        } else {
                            snapshot.events.push(Event {
                                id: mk_event_id(
                                    &cycle_id,
                                    &service_eid,
                                    &format!("dependency_service_unresolved_{}", target),
                                ),
                                kind: "service_dependency_unresolved".to_string(),
                                severity: "info".to_string(),
                                target: service_eid.clone(),
                                message: format!(
                                    "Service '{}' declares dependency on service '{}' but no matching service entity was found",
                                    item.name, target
                                ),
                                observed_at_utc: observed_at,
                            });
                        }
                    }

                    _ => {
                        snapshot.events.push(Event {
                            id: mk_event_id(
                                &cycle_id,
                                &service_eid,
                                &format!("dependency_kind_unsupported_{}", dep.kind),
                            ),
                            kind: "service_dependency_unresolved".to_string(),
                            severity: "info".to_string(),
                            target: service_eid.clone(),
                            message: format!(
                                "Service '{}' declares unsupported dependency kind '{}'",
                                item.name, dep.kind
                            ),
                            observed_at_utc: observed_at,
                        });
                    }
                }
            }
        }

        Ok(())
    }
}