use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use crate::core::model::{Entity, Snapshot};

use super::model::{LocalDGraphSnapshot, LocalHealthSnapshot, LocalModuleSnapshot};

const OBSERVATION_SOURCE: &str = "persisted_local_snapshot";
const MAX_FUTURE_SKEW_SECONDS: i64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservedHealthClass {
    Healthy,
    Unknown,
    Degraded,
    Unavailable,
}

impl ObservedHealthClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Unknown => "unknown",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
        }
    }
}

pub fn apply_persisted_observed_health(
    dgraph: &mut LocalDGraphSnapshot,
    observed: &Snapshot,
    evaluated_at: DateTime<Utc>,
) -> Result<()> {
    if dgraph.node_id.trim() != observed.meta.node_id.trim() {
        bail!(
            "observed-health snapshot node_id mismatch: dgraph={} observed={}",
            dgraph.node_id,
            observed.meta.node_id
        );
    }

    let module_ids = dgraph
        .modules
        .iter()
        .map(|module| module.module_id.clone())
        .collect::<Vec<_>>();

    for module_id in module_ids {
        let observation = find_freshest_service_observation(observed, &module_id);
        let update = build_observed_health_update(&module_id, observation, observed, evaluated_at);

        if let Some(module) = dgraph
            .modules
            .iter_mut()
            .find(|module| module.module_id == module_id)
        {
            apply_module_update(module, &update);
        }

        if let Some(health) = dgraph.health.iter_mut().find(|health| {
            health.target_kind.eq_ignore_ascii_case("module")
                && health.target_id == module_id
        }) {
            *health = update.health;
        } else {
            dgraph.health.push(update.health);
        }
    }

    merge_metadata_object(
        &mut dgraph.metadata,
        "observed_health",
        json!({
            "enabled": true,
            "source": OBSERVATION_SOURCE,
            "snapshot_cycle_id": observed.meta.cycle_id,
            "snapshot_observed_at_utc": observed.meta.observed_at_utc.to_rfc3339(),
            "evaluated_at_utc": evaluated_at.to_rfc3339(),
            "read_only": true,
            "checks_executed": false,
            "side_effects_performed": false,
        }),
    );

    dgraph.safe_to_auto_execute = false;
    dgraph.side_effects_performed = false;
    Ok(())
}

struct ObservedHealthUpdate {
    status: &'static str,
    health: LocalHealthSnapshot,
    metadata: Value,
}

fn build_observed_health_update(
    module_id: &str,
    observation: Option<&Entity>,
    observed: &Snapshot,
    evaluated_at: DateTime<Utc>,
) -> ObservedHealthUpdate {
    let Some(entity) = observation else {
        let metadata = json!({
            "reporting_only": true,
            "observation_present": false,
            "observation_fresh": false,
            "snapshot_cycle_id": observed.meta.cycle_id,
            "checks_executed": false,
        });
        return ObservedHealthUpdate {
            status: "unknown",
            health: LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: module_id.to_string(),
                status: "unknown".to_string(),
                reason: Some(
                    "no matching service observation found in persisted local snapshot"
                        .to_string(),
                ),
                source: OBSERVATION_SOURCE.to_string(),
                metadata: Some(metadata.clone()),
            },
            metadata,
        };
    };

    let raw_status = entity
        .attrs
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let age = evaluated_at.signed_duration_since(entity.observed_at_utc);
    let age_seconds = age.num_seconds().max(0) as u64;
    let clock_skew_detected = age.num_seconds() < -MAX_FUTURE_SKEW_SECONDS;
    let observation_fresh = !clock_skew_detected && age_seconds <= entity.ttl_s;
    let class = if observation_fresh {
        classify_observed_status(raw_status)
    } else {
        ObservedHealthClass::Unknown
    };
    let status = class.as_str();
    let reason = if clock_skew_detected {
        "persisted service observation is in the future beyond allowed clock skew"
    } else if !observation_fresh {
        "persisted service observation exceeded its ttl_s"
    } else if class == ObservedHealthClass::Unknown {
        "persisted service observation did not report a recognized explicit health state"
    } else {
        "derived from a fresh persisted service observation without executing checks"
    };
    let metadata = json!({
        "reporting_only": true,
        "observation_present": true,
        "observation_fresh": observation_fresh,
        "observed_at_utc": entity.observed_at_utc.to_rfc3339(),
        "evaluated_at_utc": evaluated_at.to_rfc3339(),
        "ttl_s": entity.ttl_s,
        "age_s": age_seconds,
        "clock_skew_detected": clock_skew_detected,
        "raw_status": raw_status,
        "snapshot_cycle_id": observed.meta.cycle_id,
        "service_entity_id": entity.id,
        "checks_executed": false,
    });

    ObservedHealthUpdate {
        status,
        health: LocalHealthSnapshot {
            target_kind: "module".to_string(),
            target_id: module_id.to_string(),
            status: status.to_string(),
            reason: Some(reason.to_string()),
            source: OBSERVATION_SOURCE.to_string(),
            metadata: Some(metadata.clone()),
        },
        metadata,
    }
}

fn apply_module_update(module: &mut LocalModuleSnapshot, update: &ObservedHealthUpdate) {
    module.observed_state = Some(update.status.to_string());
    merge_metadata_object(
        &mut module.metadata,
        "observed_health",
        update.metadata.clone(),
    );
}

fn find_freshest_service_observation<'a>(
    observed: &'a Snapshot,
    module_id: &str,
) -> Option<&'a Entity> {
    observed
        .entities
        .iter()
        .filter(|entity| {
            entity.kind.eq_ignore_ascii_case("service")
                && entity
                    .attrs
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.trim().eq_ignore_ascii_case(module_id.trim()))
        })
        .max_by_key(|entity| entity.observed_at_utc)
}

fn classify_observed_status(status: &str) -> ObservedHealthClass {
    match status.trim().to_ascii_lowercase().as_str() {
        "available" | "healthy" | "ok" | "running" => ObservedHealthClass::Healthy,
        "degraded" | "warning" => ObservedHealthClass::Degraded,
        "unavailable" | "down" | "failed" | "critical" | "missing" => {
            ObservedHealthClass::Unavailable
        }
        _ => ObservedHealthClass::Unknown,
    }
}

fn merge_metadata_object(metadata: &mut Option<Value>, key: &str, value: Value) {
    let object = metadata
        .get_or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut();

    if let Some(object) = object {
        object.insert(key.to_string(), value);
    } else {
        let mut replacement = Map::new();
        replacement.insert(key.to_string(), value);
        *metadata = Some(Value::Object(replacement));
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use crate::{
        core::model::{Entity, Meta, Snapshot},
        dgraph_snapshot::model::{LocalDGraphSnapshot, LocalHealthSnapshot, LocalModuleSnapshot},
    };

    use super::apply_persisted_observed_health;

    fn evaluated_at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 31, 21, 0, 0)
            .single()
            .expect("valid time")
    }

    fn dgraph(node_id: &str) -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            node_id: node_id.to_string(),
            modules: vec![LocalModuleSnapshot {
                module_id: "mqtt-adapter".to_string(),
                observed_state: Some("unknown".to_string()),
                metadata: Some(json!({ "configured": true })),
                ..LocalModuleSnapshot::default()
            }],
            health: vec![LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: "mqtt-adapter".to_string(),
                status: "unknown".to_string(),
                reason: None,
                source: "local_config".to_string(),
                metadata: None,
            }],
            safe_to_auto_execute: false,
            side_effects_performed: false,
            ..LocalDGraphSnapshot::default()
        }
    }

    fn observed(node_id: &str, status: &str, age_s: i64, ttl_s: u64) -> Snapshot {
        let observed_at = evaluated_at() - chrono::Duration::seconds(age_s);
        Snapshot {
            meta: Meta {
                schema_version: "1".to_string(),
                agent_version: "0.1.0".to_string(),
                cycle_id: "cycle-observed-health".to_string(),
                observed_at_utc: observed_at,
                node_id: node_id.to_string(),
                host_id: "host:test".to_string(),
            },
            entities: vec![Entity {
                id: "service:mqtt-adapter".to_string(),
                kind: "service".to_string(),
                attrs: json!({
                    "name": "MQTT-Adapter",
                    "status": status,
                }),
                observed_at_utc: observed_at,
                ttl_s,
            }],
            edges: vec![],
            events: vec![],
            actions: vec![],
        }
    }

    #[test]
    fn fresh_available_observation_becomes_healthy_without_side_effects() {
        let mut dgraph = dgraph("fog-02");
        let observed = observed("fog-02", "available", 10, 60);

        apply_persisted_observed_health(&mut dgraph, &observed, evaluated_at())
            .expect("observation should apply");

        assert_eq!(dgraph.modules[0].observed_state.as_deref(), Some("healthy"));
        assert_eq!(dgraph.health[0].status, "healthy");
        assert_eq!(dgraph.health[0].source, "persisted_local_snapshot");
        assert_eq!(
            dgraph.health[0]
                .metadata
                .as_ref()
                .and_then(|value| value.get("checks_executed"))
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert!(!dgraph.safe_to_auto_execute);
        assert!(!dgraph.side_effects_performed);
    }

    #[test]
    fn fresh_explicit_failure_classes_are_preserved() {
        for (raw, expected) in [
            ("degraded", "degraded"),
            ("warning", "degraded"),
            ("unavailable", "unavailable"),
            ("failed", "unavailable"),
        ] {
            let mut dgraph = dgraph("fog-02");
            let observed = observed("fog-02", raw, 10, 60);

            apply_persisted_observed_health(&mut dgraph, &observed, evaluated_at())
                .expect("observation should apply");

            assert_eq!(dgraph.health[0].status, expected);
        }
    }

    #[test]
    fn stale_unavailable_observation_becomes_unknown() {
        let mut dgraph = dgraph("fog-02");
        let observed = observed("fog-02", "unavailable", 61, 60);

        apply_persisted_observed_health(&mut dgraph, &observed, evaluated_at())
            .expect("stale observation should safely apply as unknown");

        assert_eq!(dgraph.modules[0].observed_state.as_deref(), Some("unknown"));
        assert_eq!(dgraph.health[0].status, "unknown");
        assert!(dgraph.health[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("ttl_s")));
    }

    #[test]
    fn future_observation_beyond_clock_skew_becomes_unknown() {
        let mut dgraph = dgraph("fog-02");
        let observed = observed("fog-02", "available", -6, 60);

        apply_persisted_observed_health(&mut dgraph, &observed, evaluated_at())
            .expect("future observation should safely apply as unknown");

        assert_eq!(dgraph.health[0].status, "unknown");
        assert_eq!(
            dgraph.health[0]
                .metadata
                .as_ref()
                .and_then(|value| value.get("clock_skew_detected"))
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn node_mismatch_is_rejected() {
        let mut dgraph = dgraph("fog-02");
        let observed = observed("fog-01", "available", 10, 60);

        let error = apply_persisted_observed_health(&mut dgraph, &observed, evaluated_at())
            .expect_err("cross-node observation must be rejected");

        assert!(error.to_string().contains("node_id mismatch"));
    }

    #[test]
    fn source_guard_confirms_read_only_translation_only() {
        let source = include_str!("observed_health.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "reqwest",
            "TcpStream",
            "fs::write",
            "OpenOptions",
            "run_cycle",
            "run_http_health",
            "run_tcp_port",
            "run_process_alive",
            "docker run",
            "docker inspect",
            "run_embedded_wasm",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "observed health translation must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
