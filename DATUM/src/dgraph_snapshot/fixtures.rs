use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};

use crate::config::Config;

use super::{
    builder::build_local_dgraph_snapshot,
    model::{
        LocalCapabilitySnapshot, LocalDGraphSnapshot, LocalFlowEdgeSnapshot, LocalFlowSnapshot,
        LocalHealthSnapshot, LocalModuleSnapshot, LocalRuntimeSnapshot,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DGraphSnapshotFixtureScenario {
    Baseline,
    DegradedNode,
    UnavailableNode,
    MissingModule,
    DegradedFlow,
    NoCandidate,
}

impl DGraphSnapshotFixtureScenario {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "baseline" => Ok(Self::Baseline),
            "degraded-node" => Ok(Self::DegradedNode),
            "unavailable-node" => Ok(Self::UnavailableNode),
            "missing-module" => Ok(Self::MissingModule),
            "degraded-flow" => Ok(Self::DegradedFlow),
            "no-candidate" => Ok(Self::NoCandidate),
            _ => Err(anyhow!(
                "unknown d-graph fixture scenario '{value}'. Expected one of: baseline, degraded-node, unavailable-node, missing-module, degraded-flow, no-candidate"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::DegradedNode => "degraded-node",
            Self::UnavailableNode => "unavailable-node",
            Self::MissingModule => "missing-module",
            Self::DegradedFlow => "degraded-flow",
            Self::NoCandidate => "no-candidate",
        }
    }
}

pub fn build_dgraph_fixture_snapshot(
    config: &Config,
    scenario: DGraphSnapshotFixtureScenario,
) -> LocalDGraphSnapshot {
    let mut snapshot = build_local_dgraph_snapshot(config);
    ensure_fixture_baseline_entities(&mut snapshot);

    if matches!(
        scenario,
        DGraphSnapshotFixtureScenario::Baseline | DGraphSnapshotFixtureScenario::DegradedFlow
    ) {
        ensure_cross_node_flow_fixture(&mut snapshot);
    }

    match scenario {
        DGraphSnapshotFixtureScenario::Baseline => {}
        DGraphSnapshotFixtureScenario::DegradedNode => {
            upsert_health(
                &mut snapshot.health,
                LocalHealthSnapshot {
                    target_kind: "node".to_string(),
                    target_id: snapshot.node_id.clone(),
                    status: "degraded".to_string(),
                    reason: Some("fixture degraded node scenario".to_string()),
                    source: "fixture".to_string(),
                    metadata: Some(json!({ "fixture": true })),
                },
            );
        }
        DGraphSnapshotFixtureScenario::UnavailableNode => {
            upsert_health(
                &mut snapshot.health,
                LocalHealthSnapshot {
                    target_kind: "node".to_string(),
                    target_id: snapshot.node_id.clone(),
                    status: "unavailable".to_string(),
                    reason: Some("fixture unavailable node scenario".to_string()),
                    source: "fixture".to_string(),
                    metadata: Some(json!({ "fixture": true })),
                },
            );
        }
        DGraphSnapshotFixtureScenario::MissingModule => {
            let module_id = ensure_module(&mut snapshot);
            if let Some(module) = snapshot
                .modules
                .iter_mut()
                .find(|m| m.module_id == module_id)
            {
                module.desired_state = Some("running".to_string());
                module.observed_state = Some("missing".to_string());
                merge_metadata(
                    &mut module.metadata,
                    json!({
                        "fixture": true,
                        "fixture_note": "module intentionally marked missing"
                    }),
                );
            }

            upsert_health(
                &mut snapshot.health,
                LocalHealthSnapshot {
                    target_kind: "module".to_string(),
                    target_id: module_id,
                    status: "missing".to_string(),
                    reason: Some("fixture missing module scenario".to_string()),
                    source: "fixture".to_string(),
                    metadata: Some(json!({ "fixture": true })),
                },
            );
        }
        DGraphSnapshotFixtureScenario::DegradedFlow => {
            let flow_id = ensure_cross_node_flow_fixture(&mut snapshot)
                .unwrap_or_else(|| ensure_flow(&mut snapshot));
            if let Some(flow) = snapshot
                .flows
                .iter_mut()
                .find(|flow| flow.flow_id == flow_id)
            {
                flow.status = Some("degraded".to_string());
                merge_metadata(
                    &mut flow.metadata,
                    json!({
                        "fixture": true,
                        "fixture_note": "flow intentionally marked degraded"
                    }),
                );
                if let Some(edge) = flow.edges.first_mut() {
                    edge.status = Some("degraded".to_string());
                    merge_metadata(
                        &mut edge.metadata,
                        json!({
                            "fixture": true,
                            "fixture_note": "flow edge intentionally marked degraded"
                        }),
                    );
                }
            }

            upsert_health(
                &mut snapshot.health,
                LocalHealthSnapshot {
                    target_kind: "flow".to_string(),
                    target_id: flow_id,
                    status: "degraded".to_string(),
                    reason: Some("fixture degraded flow scenario".to_string()),
                    source: "fixture".to_string(),
                    metadata: Some(json!({ "fixture": true })),
                },
            );
        }
        DGraphSnapshotFixtureScenario::NoCandidate => {
            let module_id = ensure_module(&mut snapshot);
            let runtime_id = ensure_runtime(&mut snapshot);

            if let Some(runtime) = snapshot
                .runtimes
                .iter_mut()
                .find(|runtime| runtime.runtime_id == runtime_id)
            {
                runtime.status = "unavailable".to_string();
                merge_metadata(
                    &mut runtime.metadata,
                    json!({
                        "fixture": true,
                        "fixture_note": "runtime intentionally unavailable for no-candidate scenario"
                    }),
                );
            }

            if let Some(module) = snapshot
                .modules
                .iter_mut()
                .find(|m| m.module_id == module_id)
            {
                module.desired_state = Some("running".to_string());
                module.observed_state = Some("missing".to_string());
                module.runtime = Some("container".to_string());
                merge_metadata(
                    &mut module.metadata,
                    json!({
                        "fixture": true,
                        "fixture_note": "module requires unavailable runtime"
                    }),
                );
            }

            for capability in &mut snapshot.capabilities {
                if capability.capability_kind == "container_runtime_configured" {
                    capability.supported = false;
                    merge_metadata(
                        &mut capability.metadata,
                        json!({
                            "fixture": true,
                            "fixture_note": "container runtime support intentionally disabled"
                        }),
                    );
                }
            }

            upsert_health(
                &mut snapshot.health,
                LocalHealthSnapshot {
                    target_kind: "runtime".to_string(),
                    target_id: runtime_id,
                    status: "unavailable".to_string(),
                    reason: Some("fixture no candidate scenario".to_string()),
                    source: "fixture".to_string(),
                    metadata: Some(json!({ "fixture": true })),
                },
            );
            upsert_health(
                &mut snapshot.health,
                LocalHealthSnapshot {
                    target_kind: "module".to_string(),
                    target_id: module_id,
                    status: "missing".to_string(),
                    reason: Some("fixture no candidate scenario".to_string()),
                    source: "fixture".to_string(),
                    metadata: Some(json!({ "fixture": true })),
                },
            );
        }
    }

    mark_fixture_metadata(&mut snapshot, scenario);
    snapshot.safe_to_auto_execute = false;
    snapshot.side_effects_performed = false;
    snapshot
}

fn ensure_cross_node_flow_fixture(snapshot: &mut LocalDGraphSnapshot) -> Option<String> {
    match snapshot.node_id.as_str() {
        "fog-01" => Some(ensure_cross_node_flow(
            snapshot,
            "flow-chirpstack-to-mqtt-adapter",
            "chirpstack-primary",
            "fog-01",
            "fixture-chirpstack-primary",
            "mqtt-adapter",
            "fog-02",
            "fixture-mqtt-adapter",
            "mqtt",
        )),
        "fog-02" => Some(ensure_cross_node_flow(
            snapshot,
            "flow-mqtt-adapter-to-magistrala-core",
            "mqtt-adapter",
            "fog-02",
            "fixture-mqtt-adapter",
            "magistrala-core",
            "cloud-01",
            "fixture-magistrala-core",
            "mqtt",
        )),
        "cloud-01" => {
            ensure_named_fixture_module(
                snapshot,
                "magistrala-core",
                "cloud-01",
                "fixture-magistrala-core",
            );
            None
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn ensure_cross_node_flow(
    snapshot: &mut LocalDGraphSnapshot,
    flow_id: &str,
    source_module_id: &str,
    source_node_id: &str,
    source_mapping_key: &str,
    target_module_id: &str,
    target_node_id: &str,
    target_mapping_key: &str,
    protocol: &str,
) -> String {
    let source_module_id = ensure_named_fixture_module(
        snapshot,
        source_module_id,
        source_node_id,
        source_mapping_key,
    );
    let target_module_id = ensure_named_fixture_module(
        snapshot,
        target_module_id,
        target_node_id,
        target_mapping_key,
    );
    let flow_id = flow_id.to_string();
    let edge_id = format!(
        "edge:{}:{}:{}",
        source_module_id, target_module_id, protocol
    );
    let flow_metadata = json!({
        "fixture": true,
        "reporting_only": true,
        "cross_node": true,
        "source_node_id": source_node_id,
        "target_node_id": target_node_id,
    });

    if let Some(flow) = snapshot
        .flows
        .iter_mut()
        .find(|flow| flow.flow_id == flow_id)
    {
        flow.flow_kind = Some("application_data".to_string());
        flow.modules = vec![source_module_id.clone(), target_module_id.clone()];
        flow.status = Some("configured".to_string());
        flow.source = "fixture".to_string();
        merge_metadata(&mut flow.metadata, flow_metadata.clone());
        if !flow.edges.iter().any(|edge| edge.edge_id == edge_id) {
            flow.edges.push(LocalFlowEdgeSnapshot {
                edge_id,
                from_module_id: source_module_id,
                to_module_id: target_module_id,
                protocol: Some(protocol.to_string()),
                status: Some("configured".to_string()),
                metadata: Some(flow_metadata),
            });
        }
        return flow_id;
    }

    snapshot.flows.push(LocalFlowSnapshot {
        flow_id: flow_id.clone(),
        flow_kind: Some("application_data".to_string()),
        modules: vec![source_module_id.clone(), target_module_id.clone()],
        edges: vec![LocalFlowEdgeSnapshot {
            edge_id,
            from_module_id: source_module_id,
            to_module_id: target_module_id,
            protocol: Some(protocol.to_string()),
            status: Some("configured".to_string()),
            metadata: Some(flow_metadata.clone()),
        }],
        status: Some("configured".to_string()),
        source: "fixture".to_string(),
        metadata: Some(flow_metadata),
    });
    flow_id
}

fn ensure_named_fixture_module(
    snapshot: &mut LocalDGraphSnapshot,
    module_id: &str,
    node_id: &str,
    local_mapping_key: &str,
) -> String {
    let metadata = json!({
        "fixture": true,
        "reporting_only": true,
        "node_id": node_id,
    });
    if let Some(module) = snapshot
        .modules
        .iter_mut()
        .find(|module| module.module_id == module_id)
    {
        merge_metadata(&mut module.metadata, metadata);
        return module_id.to_string();
    }

    snapshot.modules.push(LocalModuleSnapshot {
        module_id: module_id.to_string(),
        module_kind: Some("service".to_string()),
        desired_state: Some("running".to_string()),
        observed_state: Some("unknown".to_string()),
        runtime: Some("container".to_string()),
        artifact_ref: None,
        local_mapping_key: Some(local_mapping_key.to_string()),
        local_container_name: None,
        source: "fixture".to_string(),
        metadata: Some(metadata),
    });
    module_id.to_string()
}

fn ensure_fixture_baseline_entities(snapshot: &mut LocalDGraphSnapshot) {
    let _ = ensure_runtime(snapshot);
    let _ = ensure_module(snapshot);

    if snapshot.health.is_empty() {
        let module_id = snapshot.modules[0].module_id.clone();
        snapshot.health.push(LocalHealthSnapshot {
            target_kind: "module".to_string(),
            target_id: module_id,
            status: "ok".to_string(),
            reason: Some("fixture baseline default health".to_string()),
            source: "fixture".to_string(),
            metadata: Some(json!({ "fixture": true })),
        });
    }

    if snapshot.capabilities.is_empty() {
        snapshot.capabilities.push(LocalCapabilitySnapshot {
            capability_id: "capability-container-runtime-configured".to_string(),
            capability_kind: "container_runtime_configured".to_string(),
            supported: true,
            source: "fixture".to_string(),
            metadata: Some(json!({ "fixture": true })),
        });
    }
}

fn ensure_runtime(snapshot: &mut LocalDGraphSnapshot) -> String {
    if let Some(runtime) = snapshot.runtimes.first() {
        return runtime.runtime_id.clone();
    }

    let runtime_id = "runtime-container-local".to_string();
    snapshot.runtimes.push(LocalRuntimeSnapshot {
        runtime_id: runtime_id.clone(),
        runtime_kind: "container".to_string(),
        status: "configured".to_string(),
        source: "fixture".to_string(),
        metadata: Some(json!({
            "fixture": true,
            "reporting_only": true
        })),
    });
    runtime_id
}

fn ensure_module(snapshot: &mut LocalDGraphSnapshot) -> String {
    if let Some(module) = snapshot.modules.first() {
        return module.module_id.clone();
    }

    let module_id = "fixture-module-primary".to_string();
    snapshot.modules.push(LocalModuleSnapshot {
        module_id: module_id.clone(),
        module_kind: Some("service".to_string()),
        desired_state: Some("running".to_string()),
        observed_state: Some("unknown".to_string()),
        runtime: Some("container".to_string()),
        artifact_ref: None,
        local_mapping_key: Some("fixture-module-primary".to_string()),
        local_container_name: Some("fixture-container-primary".to_string()),
        source: "fixture".to_string(),
        metadata: Some(json!({
            "fixture": true,
            "reporting_only": true
        })),
    });
    module_id
}

fn ensure_flow(snapshot: &mut LocalDGraphSnapshot) -> String {
    if !snapshot.flows.is_empty() {
        let flow_id = snapshot.flows[0].flow_id.clone();
        let needs_population =
            snapshot.flows[0].edges.is_empty() || snapshot.flows[0].modules.is_empty();
        if needs_population {
            let module_ids = ensure_flow_modules(snapshot);
            if let Some(existing_flow) = snapshot.flows.first_mut() {
                if existing_flow.modules.is_empty() {
                    existing_flow.modules = module_ids.clone();
                }
                if existing_flow.edges.is_empty() {
                    existing_flow.edges.push(LocalFlowEdgeSnapshot {
                        edge_id: "edge:fixture-module-primary:fixture-module-secondary:internal"
                            .to_string(),
                        from_module_id: module_ids[0].clone(),
                        to_module_id: module_ids[1].clone(),
                        protocol: Some("internal".to_string()),
                        status: Some("configured".to_string()),
                        metadata: Some(json!({ "fixture": true })),
                    });
                }
                if existing_flow.status.is_none() {
                    existing_flow.status = Some("configured".to_string());
                }
            }
        }
        return flow_id;
    }

    let module_ids = ensure_flow_modules(snapshot);
    let flow_id = "fixture-flow-primary".to_string();
    snapshot.flows.push(LocalFlowSnapshot {
        flow_id: flow_id.clone(),
        flow_kind: Some("fixture".to_string()),
        modules: module_ids.clone(),
        edges: vec![LocalFlowEdgeSnapshot {
            edge_id: "edge:fixture-module-primary:fixture-module-secondary:internal".to_string(),
            from_module_id: module_ids[0].clone(),
            to_module_id: module_ids[1].clone(),
            protocol: Some("internal".to_string()),
            status: Some("configured".to_string()),
            metadata: Some(json!({ "fixture": true })),
        }],
        status: Some("configured".to_string()),
        source: "fixture".to_string(),
        metadata: Some(json!({
            "fixture": true,
            "reporting_only": true
        })),
    });
    flow_id
}

fn ensure_flow_modules(snapshot: &mut LocalDGraphSnapshot) -> Vec<String> {
    let primary = ensure_module(snapshot);
    let secondary = if snapshot.modules.len() >= 2 {
        snapshot.modules[1].module_id.clone()
    } else {
        let module_id = "fixture-module-secondary".to_string();
        snapshot.modules.push(LocalModuleSnapshot {
            module_id: module_id.clone(),
            module_kind: Some("service".to_string()),
            desired_state: Some("running".to_string()),
            observed_state: Some("unknown".to_string()),
            runtime: Some("container".to_string()),
            artifact_ref: None,
            local_mapping_key: Some("fixture-module-secondary".to_string()),
            local_container_name: Some("fixture-container-secondary".to_string()),
            source: "fixture".to_string(),
            metadata: Some(json!({
                "fixture": true,
                "reporting_only": true
            })),
        });
        module_id
    };

    vec![primary, secondary]
}

fn upsert_health(health: &mut Vec<LocalHealthSnapshot>, incoming: LocalHealthSnapshot) {
    if let Some(existing) = health.iter_mut().find(|entry| {
        entry.target_kind == incoming.target_kind && entry.target_id == incoming.target_id
    }) {
        *existing = incoming;
        return;
    }

    health.push(incoming);
}

fn mark_fixture_metadata(
    snapshot: &mut LocalDGraphSnapshot,
    scenario: DGraphSnapshotFixtureScenario,
) {
    merge_metadata(
        &mut snapshot.metadata,
        json!({
            "fixture": true,
            "fixture_scenario": scenario.as_str(),
        }),
    );
}

fn merge_metadata(target: &mut Option<Value>, addition: Value) {
    let mut merged = match target.take() {
        Some(Value::Object(map)) => map,
        Some(value) => {
            let mut map = Map::new();
            map.insert("previous".to_string(), value);
            map
        }
        None => Map::new(),
    };

    if let Value::Object(addition_map) = addition {
        for (key, value) in addition_map {
            merged.insert(key, value);
        }
    }

    *target = Some(Value::Object(merged));
}

#[cfg(test)]
mod tests {
    use super::{build_dgraph_fixture_snapshot, DGraphSnapshotFixtureScenario};
    use crate::config::{
        AgentConfig, Config, DesiredServiceSpec, DesiredStateConfig, FileJsonConfig, OutputsConfig,
        ServiceDependencySpec, ServiceRegistryConfig, ServiceRegistryService, ServiceSelector,
    };

    fn base_config() -> Config {
        Config {
            agent: AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "0.0.1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: OutputsConfig {
                file_json: Some(FileJsonConfig {
                    enabled: true,
                    path: "snapshot.json".to_string(),
                }),
                http: None,
            },
            dispatch: None,
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: Some(ServiceRegistryConfig {
                enabled: true,
                services: vec![
                    ServiceRegistryService {
                        name: "chirpstack-primary".to_string(),
                        runtime: "container".to_string(),
                        selector: Some(ServiceSelector {
                            container_name: Some("chirpstack".to_string()),
                            process_name: None,
                        }),
                        recovery: None,
                        checks: vec![],
                        depends_on: vec![ServiceDependencySpec {
                            kind: "mqtt".to_string(),
                            target: Some("mqtt-adapter".to_string()),
                        }],
                    },
                    ServiceRegistryService {
                        name: "mqtt-adapter".to_string(),
                        runtime: "container".to_string(),
                        selector: Some(ServiceSelector {
                            container_name: Some("mqtt-adapter".to_string()),
                            process_name: None,
                        }),
                        recovery: None,
                        checks: vec![],
                        depends_on: vec![],
                    },
                ],
            }),
            auto_recovery: None,
            desired_state: Some(DesiredStateConfig {
                enabled: true,
                services: vec![
                    DesiredServiceSpec {
                        name: "chirpstack-primary".to_string(),
                        desired: "running".to_string(),
                    },
                    DesiredServiceSpec {
                        name: "mqtt-adapter".to_string(),
                        desired: "running".to_string(),
                    },
                ],
            }),
        }
    }

    fn sparse_config() -> Config {
        let mut config = base_config();
        config.service_registry = None;
        config.desired_state = None;
        config
    }

    #[test]
    fn baseline_fixture_produces_safe_to_auto_execute_false() {
        let snapshot = build_dgraph_fixture_snapshot(
            &sparse_config(),
            DGraphSnapshotFixtureScenario::Baseline,
        );
        assert!(!snapshot.safe_to_auto_execute);
    }

    #[test]
    fn baseline_fixture_produces_side_effects_performed_false() {
        let snapshot = build_dgraph_fixture_snapshot(
            &sparse_config(),
            DGraphSnapshotFixtureScenario::Baseline,
        );
        assert!(!snapshot.side_effects_performed);
    }

    #[test]
    fn fog_01_baseline_fixture_includes_metadata_only_cross_node_flow() {
        let snapshot = build_dgraph_fixture_snapshot(
            &sparse_config(),
            DGraphSnapshotFixtureScenario::Baseline,
        );
        let flow = snapshot
            .flows
            .iter()
            .find(|flow| flow.flow_id == "flow-chirpstack-to-mqtt-adapter")
            .expect("fog-01 baseline should include the cross-node fixture flow");

        assert_eq!(flow.flow_kind.as_deref(), Some("application_data"));
        assert_eq!(
            flow.modules,
            vec!["chirpstack-primary".to_string(), "mqtt-adapter".to_string()]
        );
        assert!(flow.edges.iter().any(|edge| {
            edge.from_module_id == "chirpstack-primary"
                && edge.to_module_id == "mqtt-adapter"
                && edge.protocol.as_deref() == Some("mqtt")
        }));
        assert_eq!(
            flow.metadata
                .as_ref()
                .and_then(|metadata| metadata.get("cross_node")),
            Some(&serde_json::json!(true))
        );
        assert!(!snapshot.safe_to_auto_execute);
        assert!(!snapshot.side_effects_performed);
    }

    #[test]
    fn fog_02_baseline_fixture_includes_metadata_only_cloud_flow() {
        let mut config = sparse_config();
        config.agent.node_id = "fog-02".to_string();
        let snapshot =
            build_dgraph_fixture_snapshot(&config, DGraphSnapshotFixtureScenario::Baseline);
        let flow = snapshot
            .flows
            .iter()
            .find(|flow| flow.flow_id == "flow-mqtt-adapter-to-magistrala-core")
            .expect("fog-02 baseline should include the cloud-bound fixture flow");

        assert_eq!(
            flow.modules,
            vec!["mqtt-adapter".to_string(), "magistrala-core".to_string()]
        );
        assert!(flow.edges.iter().any(|edge| {
            edge.from_module_id == "mqtt-adapter"
                && edge.to_module_id == "magistrala-core"
                && edge.protocol.as_deref() == Some("mqtt")
        }));
        assert_eq!(
            flow.metadata
                .as_ref()
                .and_then(|metadata| metadata.get("target_node_id")),
            Some(&serde_json::json!("cloud-01"))
        );
        assert!(!snapshot.safe_to_auto_execute);
        assert!(!snapshot.side_effects_performed);
    }

    #[test]
    fn cloud_01_baseline_fixture_includes_magistrala_core_metadata() {
        let mut config = sparse_config();
        config.agent.node_id = "cloud-01".to_string();
        let snapshot =
            build_dgraph_fixture_snapshot(&config, DGraphSnapshotFixtureScenario::Baseline);
        let module = snapshot
            .modules
            .iter()
            .find(|module| module.module_id == "magistrala-core")
            .expect("cloud-01 baseline should include magistrala-core");

        assert_eq!(
            module
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("node_id")),
            Some(&serde_json::json!("cloud-01"))
        );
        assert!(!snapshot
            .flows
            .iter()
            .any(|flow| flow.flow_id == "flow-mqtt-adapter-to-magistrala-core"));
        assert!(!snapshot.safe_to_auto_execute);
        assert!(!snapshot.side_effects_performed);
    }

    #[test]
    fn degraded_node_fixture_has_node_health_status_degraded() {
        let snapshot = build_dgraph_fixture_snapshot(
            &base_config(),
            DGraphSnapshotFixtureScenario::DegradedNode,
        );

        assert!(snapshot
            .health
            .iter()
            .any(|health| health.target_kind == "node"
                && health.target_id == "fog-01"
                && health.status == "degraded"));
    }

    #[test]
    fn unavailable_node_fixture_has_node_health_status_unavailable() {
        let snapshot = build_dgraph_fixture_snapshot(
            &base_config(),
            DGraphSnapshotFixtureScenario::UnavailableNode,
        );

        assert!(snapshot
            .health
            .iter()
            .any(|health| health.target_kind == "node"
                && health.target_id == "fog-01"
                && health.status == "unavailable"));
        assert!(!snapshot.side_effects_performed);
    }

    #[test]
    fn missing_module_fixture_has_module_observed_state_missing() {
        let snapshot = build_dgraph_fixture_snapshot(
            &base_config(),
            DGraphSnapshotFixtureScenario::MissingModule,
        );

        assert!(snapshot
            .modules
            .iter()
            .any(|module| module.observed_state.as_deref() == Some("missing")));
    }

    #[test]
    fn degraded_flow_fixture_has_degraded_flow_or_edge_metadata() {
        let snapshot = build_dgraph_fixture_snapshot(
            &base_config(),
            DGraphSnapshotFixtureScenario::DegradedFlow,
        );

        let degraded_flow = snapshot
            .flows
            .iter()
            .any(|flow| flow.status.as_deref() == Some("degraded"));
        let degraded_edge = snapshot
            .flows
            .iter()
            .flat_map(|flow| flow.edges.iter())
            .any(|edge| edge.status.as_deref() == Some("degraded"));

        assert!(degraded_flow || degraded_edge);
        assert!(snapshot.flows.iter().any(|flow| {
            flow.flow_id == "flow-chirpstack-to-mqtt-adapter"
                && flow
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("cross_node"))
                    == Some(&serde_json::json!(true))
        }));
        assert!(snapshot.health.iter().any(|health| {
            health.target_kind == "flow"
                && health.target_id == "flow-chirpstack-to-mqtt-adapter"
                && health.status == "degraded"
        }));
    }

    #[test]
    fn no_candidate_fixture_creates_unavailable_or_mismatched_runtime_metadata() {
        let snapshot = build_dgraph_fixture_snapshot(
            &base_config(),
            DGraphSnapshotFixtureScenario::NoCandidate,
        );

        assert!(snapshot
            .runtimes
            .iter()
            .any(|runtime| runtime.status == "unavailable"));
        assert!(snapshot
            .capabilities
            .iter()
            .any(
                |capability| capability.capability_kind == "container_runtime_configured"
                    && !capability.supported
            ));
    }

    #[test]
    fn fixture_metadata_includes_fixture_flag_and_scenario() {
        let snapshot = build_dgraph_fixture_snapshot(
            &base_config(),
            DGraphSnapshotFixtureScenario::DegradedNode,
        );

        let metadata = snapshot.metadata.expect("fixture metadata should exist");
        assert_eq!(
            metadata.get("fixture").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(
            metadata
                .get("fixture_scenario")
                .and_then(|value| value.as_str()),
            Some("degraded-node")
        );
    }

    #[test]
    fn parse_rejects_unknown_fixture_scenario() {
        assert!(DGraphSnapshotFixtureScenario::parse("unknown-fixture").is_err());
    }

    #[test]
    fn source_guard_confirms_no_docker_shell_config_write_or_wasm_execution() {
        let source = include_str!("fixtures.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
            "run_embedded_wasm",
            "wasm_runtime::runner",
            "dispatch --execute",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "dgraph fixture builder must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
