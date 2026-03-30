use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use serde_json::Value as JsonValue;

use crate::core::ids;
use crate::core::model::{Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct ServiceTransitionsPlugin {
    last_status: Mutex<HashMap<String, String>>,
    initialized: Mutex<bool>,
}

impl ServiceTransitionsPlugin {
    pub fn new() -> Self {
        Self {
            last_status: Mutex::new(HashMap::new()),
            initialized: Mutex::new(false),
        }
    }

    fn bootstrap_from_previous_snapshot(
        &self,
        cfg: &crate::config::Config,
    ) -> Result<()> {
        let mut initialized = self
            .initialized
            .lock()
            .expect("service_transitions initialized mutex poisoned");

        if *initialized {
            return Ok(());
        }

        let snapshot_path = match cfg.outputs.file_json.as_ref() {
            Some(f) if f.enabled => f.path.clone(),
            _ => {
                *initialized = true;
                return Ok(());
            }
        };

        if !Path::new(&snapshot_path).exists() {
            *initialized = true;
            return Ok(());
        }

        let raw = match fs::read_to_string(&snapshot_path) {
            Ok(s) => s,
            Err(_) => {
                *initialized = true;
                return Ok(());
            }
        };

        let parsed: JsonValue = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => {
                *initialized = true;
                return Ok(());
            }
        };

        let entities = match parsed.get("entities").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                *initialized = true;
                return Ok(());
            }
        };

        let mut last_status = self
            .last_status
            .lock()
            .expect("service_transitions last_status mutex poisoned");

        for ent in entities {
            let kind = ent.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            if kind != "service" {
                continue;
            }

            let service_id = match ent.get("id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };

            let status = ent
                .get("attrs")
                .and_then(|a| a.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            last_status.insert(service_id, status);
        }

        *initialized = true;
        Ok(())
    }
}

fn mk_event_id(cycle_id: &str, service_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let s = ids::sanitize_id_component(service_id);
    let x = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", s, x, c)
}

impl Plugin for ServiceTransitionsPlugin {
    fn name(&self) -> &'static str {
        "service_transitions"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        self.bootstrap_from_previous_snapshot(cfg)?;

        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();

        let mut last_status = self
            .last_status
            .lock()
            .expect("service_transitions last_status mutex poisoned");

        for ent in snapshot.entities.iter() {
            if ent.kind != "service" {
                continue;
            }

            let service_id = ent.id.clone();
            let service_name = ent
                .attrs
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&service_id)
                .to_string();

            let current_status = ent
                .attrs
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let previous_status = last_status.get(&service_id).cloned();

            if let Some(prev) = previous_status {
                if prev != current_status {
                    snapshot.events.push(Event {
                        id: mk_event_id(
                            &cycle_id,
                            &service_id,
                            &format!("status_changed_{}_to_{}", prev, current_status),
                        ),
                        kind: "service_status_changed".to_string(),
                        severity: "info".to_string(),
                        target: service_id.clone(),
                        message: format!(
                            "Service '{}' changed status from '{}' to '{}'",
                            service_name, prev, current_status
                        ),
                        observed_at_utc: observed_at,
                    });

                    if prev == "unavailable" && current_status == "available" {
                        snapshot.events.push(Event {
                            id: mk_event_id(&cycle_id, &service_id, "service_recovered"),
                            kind: "service_recovered".to_string(),
                            severity: "info".to_string(),
                            target: service_id.clone(),
                            message: format!("Service '{}' recovered", service_name),
                            observed_at_utc: observed_at,
                        });
                    }

                    if current_status == "degraded" {
                        snapshot.events.push(Event {
                            id: mk_event_id(&cycle_id, &service_id, "service_degraded"),
                            kind: "service_degraded".to_string(),
                            severity: "warning".to_string(),
                            target: service_id.clone(),
                            message: format!("Service '{}' became degraded", service_name),
                            observed_at_utc: observed_at,
                        });
                    }

                    if current_status == "unavailable" {
                        snapshot.events.push(Event {
                            id: mk_event_id(&cycle_id, &service_id, "service_unavailable"),
                            kind: "service_unavailable".to_string(),
                            severity: "warning".to_string(),
                            target: service_id.clone(),
                            message: format!("Service '{}' became unavailable", service_name),
                            observed_at_utc: observed_at,
                        });
                    }
                }
            }

            // atualiza cache para o próximo ciclo
            last_status.insert(service_id, current_status);
        }

        Ok(())
    }
}