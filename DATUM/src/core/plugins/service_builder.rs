use anyhow::Result;
use serde_json::{json, Value as JsonValue};
use std::collections::HashSet;

use crate::config::{
    Config, ImplementedBy, ServiceCheckSpec, ServiceDependencySpec, ServiceRegistryService,
    ServiceSelector,
};
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

fn mk_event_id(cycle_id: &str, service_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let s = ids::sanitize_id_component(service_id);
    let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
}

fn check_spec_to_json(check: &ServiceCheckSpec) -> JsonValue {
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

fn dependency_spec_to_json(dep: &ServiceDependencySpec) -> JsonValue {
    json!({
        "kind": dep.kind,
        "target": dep.target
    })
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

    snapshot.entities.iter().find_map(|e| {
        if e.kind != "container" {
            return None;
        }

        let candidates = container_name_candidates(&e.attrs);
        if candidates.iter().any(|c| c == &wanted) {
            Some(e.id.clone())
        } else {
            None
        }
    })
}

fn find_process_group_by_name(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    snapshot.entities.iter().find_map(|e| {
        if e.kind != "process" {
            return None;
        }

        let candidates = process_name_candidates(&e.attrs);
        if candidates.iter().any(|c| c == &wanted) {
            Some(e.id.clone())
        } else {
            None
        }
    })
}

fn find_connection_by_target(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    snapshot.entities.iter().find_map(|e| {
        if e.kind != "connection" {
            return None;
        }

        let candidate = e
            .attrs
            .get("target")
            .and_then(|v| v.as_str())
            .or_else(|| e.attrs.get("to").and_then(|v| v.as_str()));

        match candidate {
            Some(target) if norm_key(target) == wanted => Some(e.id.clone()),
            _ => None,
        }
    })
}

fn find_service_by_name(snapshot: &Snapshot, wanted: &str) -> Option<String> {
    let wanted = norm_key(wanted);

    snapshot.entities.iter().find_map(|e| {
        if e.kind != "service" {
            return None;
        }

        match e.attrs.get("name").and_then(|v| v.as_str()) {
            Some(name) if norm_key(name) == wanted => Some(e.id.clone()),
            _ => None,
        }
    })
}

fn build_bindings_from_legacy(implemented_by: &[ImplementedBy]) -> Vec<JsonValue> {
    let mut out = Vec::new();

    for imp in implemented_by {
        if let Some(name) = &imp.container_name {
            out.push(json!({
                "target_kind": "container",
                "wanted": name,
                "via": "container_name",
                "bound": false,
                "target_id": null
            }));
        }

        if let Some(name) = &imp.process_name {
            out.push(json!({
                "target_kind": "process",
                "wanted": name,
                "via": "process_name",
                "bound": false,
                "target_id": null
            }));
        }
    }

    out
}

fn build_bindings_from_registry(runtime: &str, selector: &Option<ServiceSelector>) -> Vec<JsonValue> {
    let mut out = Vec::new();

    if let Some(sel) = selector {
        match runtime {
            "container" => {
                if let Some(name) = &sel.container_name {
                    out.push(json!({
                        "target_kind": "container",
                        "wanted": name,
                        "via": "container_name",
                        "bound": false,
                        "target_id": null
                    }));
                }
            }
            "process" => {
                if let Some(name) = &sel.process_name {
                    out.push(json!({
                        "target_kind": "process",
                        "wanted": name,
                        "via": "process_name",
                        "bound": false,
                        "target_id": null
                    }));
                }
            }
            _ => {}
        }
    }

    out
}

/// Resolve binding respeitando runtime do serviço.
/// - runtime=process -> só aceita process
/// - runtime=container -> só aceita container
/// - runtime ausente (legado) -> aceita conforme target_kind
fn resolve_binding_target(snapshot: &Snapshot, binding: &JsonValue, runtime: Option<&str>) -> Option<String> {
    let target_kind = binding.get("target_kind").and_then(|v| v.as_str()).unwrap_or("");
    let wanted = binding.get("wanted").and_then(|v| v.as_str()).unwrap_or("");

    if wanted.is_empty() {
        return None;
    }

    match (runtime.unwrap_or(""), target_kind) {
        ("process", "process") => find_process_group_by_name(snapshot, wanted),
        ("container", "container") => find_container_by_name(snapshot, wanted),

        // legado
        ("", "process") => find_process_group_by_name(snapshot, wanted),
        ("", "container") => find_container_by_name(snapshot, wanted),

        _ => None,
    }
}

fn edge_exists(snapshot: &Snapshot, from: &str, to: &str, rel: &str) -> bool {
    snapshot
        .edges
        .iter()
        .any(|e| e.from == from && e.to == to && e.rel == rel)
}

fn create_service_entity(
    service_name: &str,
    source: &str,
    runtime: Option<&str>,
    checks: &[ServiceCheckSpec],
    depends_on: &[ServiceDependencySpec],
    bindings: Vec<JsonValue>,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> Entity {
    let checks_declared: Vec<JsonValue> = checks.iter().map(check_spec_to_json).collect();
    let depends_on_declared: Vec<JsonValue> =
        depends_on.iter().map(dependency_spec_to_json).collect();

    Entity {
        id: ids::service_id(service_name),
        kind: "service".to_string(),
        attrs: json!({
            "name": service_name,
            "runtime": runtime,
            "status": "unknown",
            "desired_state": "running",
            "binding": {
                "bound": false
            },
            "bindings": bindings,
            "checks_declared": checks_declared,
            "depends_on_declared": depends_on_declared,
            "source": source
        }),
        observed_at_utc: observed_at,
        ttl_s: 60,
    }
}

fn add_service_from_registry(
    snapshot: &mut Snapshot,
    seen: &mut HashSet<String>,
    svc: &ServiceRegistryService,
) {
    if seen.contains(&svc.name) {
        return;
    }

    let ent = create_service_entity(
        &svc.name,
        "service_registry",
        Some(&svc.runtime),
        &svc.checks,
        &svc.depends_on,
        build_bindings_from_registry(&svc.runtime, &svc.selector),
        snapshot.meta.observed_at_utc,
    );

    seen.insert(svc.name.clone());
    snapshot.entities.push(ent);
}

fn add_service_from_legacy(
    snapshot: &mut Snapshot,
    seen: &mut HashSet<String>,
    item: &crate::config::ServiceItem,
) {
    if seen.contains(&item.name) {
        return;
    }

    let ent = create_service_entity(
        &item.name,
        "services_legacy",
        None,
        &item.checks,
        &item.depends_on,
        build_bindings_from_legacy(&item.implemented_by),
        snapshot.meta.observed_at_utc,
    );

    seen.insert(item.name.clone());
    snapshot.entities.push(ent);
}

impl Plugin for ServiceBuilderPlugin {
    fn name(&self) -> &'static str {
        "service_builder"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &Config) -> Result<()> {
        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();

        let legacy_enabled = cfg.services.as_ref().map(|s| s.enabled).unwrap_or(false);
        let registry_enabled = cfg
            .service_registry
            .as_ref()
            .map(|s| s.enabled)
            .unwrap_or(false);

        if !legacy_enabled && !registry_enabled {
            return Ok(());
        }

        let mut seen: HashSet<String> = HashSet::new();

        // PASS 1: cria entidades do registry primeiro
        for svc in cfg.registry_services() {
            add_service_from_registry(snapshot, &mut seen, svc);
        }

        // PASS 2: completa com legado, sem duplicar por nome
        if let Some(scfg) = cfg.services.as_ref() {
            if scfg.enabled {
                for item in scfg.items.iter() {
                    add_service_from_legacy(snapshot, &mut seen, item);
                }
            }
        }

        // PASS 3: resolve bindings e cria edges implemented_by
        let service_ids: Vec<String> = snapshot
            .entities
            .iter()
            .filter(|e| e.kind == "service")
            .map(|e| e.id.clone())
            .collect();

        for service_id in service_ids {
            let Some(pos) = snapshot.entities.iter().position(|e| e.id == service_id) else {
                continue;
            };

            let runtime = snapshot.entities[pos]
                .attrs
                .get("runtime")
                .and_then(|v| v.as_str());

            let service_name = snapshot.entities[pos]
                .attrs
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let current_bindings = snapshot.entities[pos]
                .attrs
                .get("bindings")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut resolved_bindings = Vec::new();
            let mut any_bound = false;

            for binding in current_bindings {
            let mut b = binding.clone();

            let target_kind = b
                .get("target_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let wanted = b
                .get("wanted")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let target_id = resolve_binding_target(snapshot, &b, runtime);

            if let Some(tid) = target_id {
                b["bound"] = json!(true);
                b["target_id"] = json!(tid.clone());
                any_bound = true;

                if !edge_exists(snapshot, &service_id, &tid, "implemented_by") {
                    snapshot.edges.push(Edge {
                        from: service_id.clone(),
                        to: tid,
                        rel: "implemented_by".to_string(),
                        attrs: json!({
                            "via": b.get("via").and_then(|v| v.as_str()).unwrap_or("unknown")
                        }),
                        observed_at_utc: observed_at,
                    });
                }
            } else {
                b["bound"] = json!(false);
                b["target_id"] = JsonValue::Null;

                snapshot.events.push(Event {
                    id: mk_event_id(
                        &cycle_id,
                        &service_id,
                        &format!("unbound_{}_{}", target_kind, wanted),
                    ),
                    kind: "service_unbound".to_string(),
                    severity: "info".to_string(),
                    target: service_id.clone(),
                    message: format!(
                        "Service '{}' has no resolved bindings for runtime {:?} (wanted {} '{}')",
                        service_name, runtime, target_kind, wanted
                    ),
                    correlation_id: None,
                    source_plugin: Some("service_builder".to_string()),
                    status: Some("open".to_string()),
                    details: Some(json!({"service_name": service_name, "runtime": runtime, "target_kind": target_kind, "wanted": wanted})),
                    observed_at_utc: observed_at,
                });
            }

            resolved_bindings.push(b);
        }

            snapshot.entities[pos].attrs["bindings"] = json!(resolved_bindings);
            snapshot.entities[pos].attrs["binding"] = json!({ "bound": any_bound });

            if !any_bound {
                snapshot.events.push(Event {
                    id: mk_event_id(&cycle_id, &service_id, "service_unbound"),
                    kind: "service_unbound".to_string(),
                    severity: "info".to_string(),
                    target: service_id.clone(),
                    message: format!(
                        "Service '{}' has no resolved bindings (check service_registry/services config)",
                        service_name
                    ),
                    correlation_id: None,
                    source_plugin: Some("service_builder".to_string()),
                    status: Some("open".to_string()),
                    details: Some(json!({"service_name": service_name, "reason": "no resolved bindings"})),
                    observed_at_utc: observed_at,
                });
            }
        }

        // PASS 4: depends_on -> edges
        let service_ids: Vec<String> = snapshot
            .entities
            .iter()
            .filter(|e| e.kind == "service")
            .map(|e| e.id.clone())
            .collect();

        for service_id in service_ids {
            let Some(pos) = snapshot.entities.iter().position(|e| e.id == service_id) else {
                continue;
            };

            let service_name = snapshot.entities[pos]
                .attrs
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let depends_on = snapshot.entities[pos]
                .attrs
                .get("depends_on_declared")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for dep in depends_on {
                let dep_kind = dep.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                let target = dep.get("target").and_then(|v| v.as_str()).unwrap_or("").trim();

                if target.is_empty() {
                    continue;
                }

                let target_id = match dep_kind {
                    "connection" => find_connection_by_target(snapshot, target),
                    "service" => find_service_by_name(snapshot, target),
                    _ => None,
                };

                if let Some(tid) = target_id {
                    if !edge_exists(snapshot, &service_id, &tid, "depends_on") {
                        snapshot.edges.push(Edge {
                            from: service_id.clone(),
                            to: tid,
                            rel: "depends_on".to_string(),
                            attrs: json!({
                                "kind": dep_kind,
                                "target": target
                            }),
                            observed_at_utc: observed_at,
                        });
                    }
                } else {
                    snapshot.events.push(Event {
                        id: mk_event_id(
                            &cycle_id,
                            &service_id,
                            &format!("dependency_unresolved_{}_{}", dep_kind, target),
                        ),
                        kind: "service_dependency_unresolved".to_string(),
                        severity: "info".to_string(),
                        target: service_id.clone(),
                        message: format!(
                            "Service '{}' has unresolved dependency kind='{}' target='{}'",
                            service_name, dep_kind, target
                        ),
                        correlation_id: None,
                        source_plugin: Some("service_builder".to_string()),
                        status: Some("open".to_string()),
                        details: Some(json!({"service_name": service_name, "dependency_kind": dep_kind, "dependency_target": target})),
                        observed_at_utc: observed_at,
                    });
                }
            }
        }

        Ok(())
    }
}