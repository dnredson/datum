use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use serde_json::json;

use crate::config::{Config, ImplementedBy, ServiceDependencySpec};

use super::model::{
    LocalCapabilitySnapshot, LocalDGraphSnapshot, LocalFlowEdgeSnapshot, LocalFlowSnapshot,
    LocalHealthSnapshot, LocalModuleSnapshot, LocalRuntimeSnapshot,
};

pub fn build_local_dgraph_snapshot(config: &Config) -> LocalDGraphSnapshot {
    let module_map = build_module_map(config);
    let modules = module_map.into_values().collect::<Vec<_>>();
    let runtimes = build_runtime_snapshots(&modules);
    let flows = build_flow_snapshots(config, &modules);
    let health = build_health_snapshots(&modules);
    let capabilities = build_capability_snapshots(config, &runtimes, &modules, &flows);

    LocalDGraphSnapshot {
        node_id: config.agent.node_id.clone(),
        project_id: resolve_project_id(config),
        snapshot_id: None,
        reported_at_utc: None,
        source: None,
        generated_at_utc: Some(Utc::now().to_rfc3339()),
        runtimes,
        modules,
        flows,
        health,
        capabilities,
        metadata: Some(json!({
            "snapshot_kind": "local_dgraph",
            "schema_version": "0.1.0",
            "reporting_only": true,
        })),
        safe_to_auto_execute: false,
        side_effects_performed: false,
    }
}

fn build_module_map(config: &Config) -> BTreeMap<String, LocalModuleSnapshot> {
    let mut modules = BTreeMap::new();

    if let Some(services) = config.services.as_ref().filter(|services| services.enabled) {
        for service in &services.items {
            let module_id = service.name.trim();
            if module_id.is_empty() {
                continue;
            }

            let runtime = infer_runtime_from_implemented_by(&service.implemented_by);
            let desired_state = config
                .find_desired_service(module_id)
                .map(|service| service.desired.clone());
            let local_container_name = service
                .implemented_by
                .iter()
                .find_map(|implementation| implementation.container_name.clone());

            modules.insert(
                module_id.to_string(),
                LocalModuleSnapshot {
                    module_id: module_id.to_string(),
                    module_kind: Some("service".to_string()),
                    desired_state,
                    observed_state: Some("unknown".to_string()),
                    runtime,
                    artifact_ref: None,
                    local_mapping_key: None,
                    local_container_name,
                    source: "services".to_string(),
                    metadata: Some(json!({
                        "implemented_by_count": service.implemented_by.len(),
                        "checks_count": service.checks.len(),
                        "dependency_count": service.depends_on.len(),
                    })),
                },
            );
        }
    }

    for service in config.registry_services() {
        let module_id = service.name.trim();
        if module_id.is_empty() {
            continue;
        }

        let desired_state = config
            .find_desired_service(module_id)
            .map(|desired| desired.desired.clone());

        let module = LocalModuleSnapshot {
            module_id: module_id.to_string(),
            module_kind: Some("service".to_string()),
            desired_state,
            observed_state: Some("unknown".to_string()),
            runtime: normalized_optional_string(Some(service.runtime.as_str())),
            artifact_ref: None,
            local_mapping_key: None,
            local_container_name: service
                .selector
                .as_ref()
                .and_then(|selector| selector.container_name.clone()),
            source: "service_registry".to_string(),
            metadata: Some(json!({
                "checks_count": service.checks.len(),
                "dependency_count": service.depends_on.len(),
                "recovery_policy": service.recovery.as_ref().map(|recovery| recovery.policy.clone()),
            })),
        };

        upsert_module(&mut modules, module);
    }

    if let Some(dispatch) = config.dispatch.as_ref() {
        for mapping in &dispatch.container_mappings {
            let module_id = normalized_optional_string(mapping.module_id.as_deref())
                .or_else(|| normalized_optional_string(mapping.service_name.as_deref()))
                .or_else(|| normalized_optional_string(Some(mapping.container_name.as_str())));

            let Some(module_id) = module_id else {
                continue;
            };

            let desired_state = config
                .find_desired_service(&module_id)
                .map(|desired| desired.desired.clone())
                .or_else(|| {
                    mapping
                        .service_name
                        .as_deref()
                        .and_then(|name| config.find_desired_service(name))
                        .map(|desired| desired.desired.clone())
                });

            let module = LocalModuleSnapshot {
                module_id: module_id.clone(),
                module_kind: Some("service".to_string()),
                desired_state,
                observed_state: Some("unknown".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: normalized_optional_string(mapping.artifact.as_deref()),
                local_mapping_key: normalized_optional_string(
                    mapping
                        .module_id
                        .as_deref()
                        .or(mapping.service_name.as_deref()),
                ),
                local_container_name: normalized_optional_string(Some(
                    mapping.container_name.as_str(),
                )),
                source: "dispatch.container_mappings".to_string(),
                metadata: Some(json!({
                    "service_name": mapping.service_name,
                    "mapped_from_dispatch": true,
                })),
            };

            upsert_module(&mut modules, module);
        }
    }

    modules
}

fn build_runtime_snapshots(modules: &[LocalModuleSnapshot]) -> Vec<LocalRuntimeSnapshot> {
    let mut runtimes = Vec::new();
    let runtime_kinds = modules
        .iter()
        .filter_map(|module| module.runtime.as_deref())
        .map(str::to_ascii_lowercase)
        .collect::<BTreeSet<_>>();

    for runtime_kind in runtime_kinds {
        runtimes.push(LocalRuntimeSnapshot {
            runtime_id: format!("runtime-{}-local", runtime_kind),
            runtime_kind: runtime_kind.clone(),
            status: "configured".to_string(),
            source: "local_config".to_string(),
            metadata: Some(json!({
                "reporting_only": true,
                "derived_from_module_count": modules
                    .iter()
                    .filter(|module| module.runtime.as_deref() == Some(runtime_kind.as_str()))
                    .count(),
            })),
        });
    }

    runtimes
}

fn build_flow_snapshots(
    config: &Config,
    modules: &[LocalModuleSnapshot],
) -> Vec<LocalFlowSnapshot> {
    let known_modules = modules
        .iter()
        .map(|module| module.module_id.clone())
        .collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    let mut flow_modules = BTreeSet::new();

    if let Some(services) = config.services.as_ref().filter(|services| services.enabled) {
        for service in &services.items {
            collect_dependency_edges(
                service.name.as_str(),
                &service.depends_on,
                "services",
                &known_modules,
                &mut edges,
                &mut flow_modules,
            );
        }
    }

    for service in config.registry_services() {
        collect_dependency_edges(
            service.name.as_str(),
            &service.depends_on,
            "service_registry",
            &known_modules,
            &mut edges,
            &mut flow_modules,
        );
    }

    if edges.is_empty() {
        return Vec::new();
    }

    vec![LocalFlowSnapshot {
        flow_id: "configured-service-flow".to_string(),
        flow_kind: Some("service_dependency".to_string()),
        modules: flow_modules.into_iter().collect(),
        edges,
        status: Some("configured".to_string()),
        source: "local_config".to_string(),
        metadata: Some(json!({
            "reporting_only": true,
        })),
    }]
}

fn build_health_snapshots(modules: &[LocalModuleSnapshot]) -> Vec<LocalHealthSnapshot> {
    modules
        .iter()
        .map(|module| LocalHealthSnapshot {
            target_kind: "module".to_string(),
            target_id: module.module_id.clone(),
            status: "unknown".to_string(),
            reason: Some("no runtime observation collected in dgraph snapshot mode".to_string()),
            source: "local_config".to_string(),
            metadata: Some(json!({
                "reporting_only": true,
            })),
        })
        .collect()
}

fn build_capability_snapshots(
    config: &Config,
    runtimes: &[LocalRuntimeSnapshot],
    modules: &[LocalModuleSnapshot],
    flows: &[LocalFlowSnapshot],
) -> Vec<LocalCapabilitySnapshot> {
    let has_container = runtimes
        .iter()
        .any(|runtime| runtime.runtime_kind == "container");
    let has_process = runtimes
        .iter()
        .any(|runtime| runtime.runtime_kind == "process");
    let has_desired_state = !config.desired_services().is_empty();
    let has_registry = !config.registry_services().is_empty();
    let has_flow_metadata = !flows.is_empty();

    vec![
        LocalCapabilitySnapshot {
            capability_id: "capability-container-runtime-configured".to_string(),
            capability_kind: "container_runtime_configured".to_string(),
            supported: has_container,
            source: "local_config".to_string(),
            metadata: Some(json!({
                "module_count": modules
                    .iter()
                    .filter(|module| module.runtime.as_deref() == Some("container"))
                    .count(),
            })),
        },
        LocalCapabilitySnapshot {
            capability_id: "capability-process-runtime-configured".to_string(),
            capability_kind: "process_runtime_configured".to_string(),
            supported: has_process,
            source: "local_config".to_string(),
            metadata: None,
        },
        LocalCapabilitySnapshot {
            capability_id: "capability-service-registry-configured".to_string(),
            capability_kind: "service_registry_configured".to_string(),
            supported: has_registry,
            source: "local_config".to_string(),
            metadata: None,
        },
        LocalCapabilitySnapshot {
            capability_id: "capability-desired-state-configured".to_string(),
            capability_kind: "desired_state_configured".to_string(),
            supported: has_desired_state,
            source: "local_config".to_string(),
            metadata: None,
        },
        LocalCapabilitySnapshot {
            capability_id: "capability-flow-metadata-configured".to_string(),
            capability_kind: "flow_metadata_configured".to_string(),
            supported: has_flow_metadata,
            source: "local_config".to_string(),
            metadata: None,
        },
    ]
}

fn resolve_project_id(config: &Config) -> Option<String> {
    config
        .dispatch
        .as_ref()
        .and_then(|dispatch| dispatch.project_id.clone())
        .or_else(|| {
            config
                .outputs
                .http
                .as_ref()
                .and_then(|http| http.project_id.clone())
        })
}

fn infer_runtime_from_implemented_by(implemented_by: &[ImplementedBy]) -> Option<String> {
    if implemented_by
        .iter()
        .any(|implementation| implementation.container_name.is_some())
    {
        return Some("container".to_string());
    }

    if implemented_by
        .iter()
        .any(|implementation| implementation.process_name.is_some())
    {
        return Some("process".to_string());
    }

    None
}

fn collect_dependency_edges(
    source_module_id: &str,
    dependencies: &[ServiceDependencySpec],
    source: &str,
    known_modules: &BTreeSet<String>,
    edges: &mut Vec<LocalFlowEdgeSnapshot>,
    flow_modules: &mut BTreeSet<String>,
) {
    for dependency in dependencies {
        let Some(target) = dependency
            .target
            .as_deref()
            .and_then(|value| normalized_optional_string(Some(value)))
        else {
            continue;
        };

        if !known_modules.contains(source_module_id) || !known_modules.contains(&target) {
            continue;
        }

        let edge_id = format!("edge:{}:{}:{}", source_module_id, target, dependency.kind);
        if edges.iter().any(|edge| edge.edge_id == edge_id) {
            continue;
        }

        flow_modules.insert(source_module_id.to_string());
        flow_modules.insert(target.clone());
        edges.push(LocalFlowEdgeSnapshot {
            edge_id,
            from_module_id: source_module_id.to_string(),
            to_module_id: target,
            protocol: normalized_optional_string(Some(dependency.kind.as_str())),
            status: Some("configured".to_string()),
            metadata: Some(json!({
                "source": source,
            })),
        });
    }
}

fn upsert_module(
    modules: &mut BTreeMap<String, LocalModuleSnapshot>,
    incoming: LocalModuleSnapshot,
) {
    if let Some(existing) = modules.get_mut(&incoming.module_id) {
        merge_module(existing, incoming);
        return;
    }

    modules.insert(incoming.module_id.clone(), incoming);
}

fn merge_module(existing: &mut LocalModuleSnapshot, incoming: LocalModuleSnapshot) {
    if existing.module_kind.is_none() {
        existing.module_kind = incoming.module_kind.clone();
    }
    if existing.desired_state.is_none() {
        existing.desired_state = incoming.desired_state.clone();
    }
    if existing.observed_state.is_none() {
        existing.observed_state = incoming.observed_state.clone();
    }
    if existing.runtime.is_none() {
        existing.runtime = incoming.runtime.clone();
    }
    if existing.artifact_ref.is_none() {
        existing.artifact_ref = incoming.artifact_ref.clone();
    }
    if existing.local_mapping_key.is_none() {
        existing.local_mapping_key = incoming.local_mapping_key.clone();
    }
    if existing.local_container_name.is_none() {
        existing.local_container_name = incoming.local_container_name.clone();
    }
    if existing.source != incoming.source {
        existing.source = "merged_config".to_string();
    }

    let existing_metadata = existing.metadata.take();
    existing.metadata = Some(json!({
        "primary": existing_metadata,
        "merged": incoming.metadata,
    }));
}

fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::build_local_dgraph_snapshot;
    use crate::config::{
        AgentConfig, Config, DesiredServiceSpec, DesiredStateConfig, DispatchConfig,
        DispatchContainerMapping, FileJsonConfig, OutputsConfig, ServiceDependencySpec,
        ServiceRegistryConfig, ServiceRegistryService, ServiceSelector,
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
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn empty_minimal_config_produces_valid_snapshot() {
        let snapshot = build_local_dgraph_snapshot(&base_config());

        assert_eq!(snapshot.node_id, "fog-01");
        assert!(snapshot.modules.is_empty());
        assert!(snapshot.flows.is_empty());
        assert!(snapshot.runtimes.is_empty());
    }

    #[test]
    fn snapshot_safe_to_auto_execute_is_false() {
        let snapshot = build_local_dgraph_snapshot(&base_config());
        assert!(!snapshot.safe_to_auto_execute);
    }

    #[test]
    fn snapshot_side_effects_performed_is_false() {
        let snapshot = build_local_dgraph_snapshot(&base_config());
        assert!(!snapshot.side_effects_performed);
    }

    #[test]
    fn service_registry_container_entry_becomes_module_snapshot() {
        let mut config = base_config();
        config.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![ServiceRegistryService {
                name: "chirpstack-primary".to_string(),
                runtime: "container".to_string(),
                selector: Some(ServiceSelector {
                    container_name: Some("chirpstack".to_string()),
                    process_name: None,
                }),
                recovery: None,
                checks: vec![],
                depends_on: vec![],
            }],
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        let module = snapshot
            .modules
            .iter()
            .find(|module| module.module_id == "chirpstack-primary")
            .expect("registry module should exist");

        assert_eq!(module.runtime.as_deref(), Some("container"));
        assert_eq!(module.local_container_name.as_deref(), Some("chirpstack"));
    }

    #[test]
    fn dispatch_container_mapping_becomes_module_snapshot() {
        let mut config = base_config();
        config.dispatch = Some(DispatchConfig {
            container_mappings: vec![DispatchContainerMapping {
                module_id: Some("local-http-ctr".to_string()),
                service_name: None,
                container_name: "test-http".to_string(),
                artifact: Some("nginx:stable".to_string()),
                ports: Vec::new(),
            }],
            ..DispatchConfig::default()
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        let module = snapshot
            .modules
            .iter()
            .find(|module| module.module_id == "local-http-ctr")
            .expect("dispatch module should exist");

        assert_eq!(module.runtime.as_deref(), Some("container"));
        assert_eq!(module.artifact_ref.as_deref(), Some("nginx:stable"));
        assert_eq!(module.local_container_name.as_deref(), Some("test-http"));
    }

    #[test]
    fn duplicate_module_ids_are_deduplicated() {
        let mut config = base_config();
        config.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![ServiceRegistryService {
                name: "local-http-ctr".to_string(),
                runtime: "container".to_string(),
                selector: Some(ServiceSelector {
                    container_name: Some("test-http".to_string()),
                    process_name: None,
                }),
                recovery: None,
                checks: vec![],
                depends_on: vec![],
            }],
        });
        config.dispatch = Some(DispatchConfig {
            container_mappings: vec![DispatchContainerMapping {
                module_id: Some("local-http-ctr".to_string()),
                service_name: None,
                container_name: "test-http".to_string(),
                artifact: Some("nginx:stable".to_string()),
                ports: Vec::new(),
            }],
            ..DispatchConfig::default()
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        assert_eq!(snapshot.modules.len(), 1);
        assert_eq!(snapshot.modules[0].module_id, "local-http-ctr");
        assert_eq!(snapshot.modules[0].source, "merged_config");
    }

    #[test]
    fn runtime_snapshot_is_produced_when_container_config_exists() {
        let mut config = base_config();
        config.dispatch = Some(DispatchConfig {
            container_mappings: vec![DispatchContainerMapping {
                module_id: Some("local-http-ctr".to_string()),
                service_name: None,
                container_name: "test-http".to_string(),
                artifact: None,
                ports: Vec::new(),
            }],
            ..DispatchConfig::default()
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        assert!(snapshot
            .runtimes
            .iter()
            .any(|runtime| runtime.runtime_kind == "container"));
    }

    #[test]
    fn health_entries_are_produced_with_safe_default_status() {
        let mut config = base_config();
        config.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![ServiceRegistryService {
                name: "local-http-ctr".to_string(),
                runtime: "container".to_string(),
                selector: None,
                recovery: None,
                checks: vec![],
                depends_on: vec![],
            }],
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        assert_eq!(snapshot.health.len(), 1);
        assert_eq!(snapshot.health[0].status, "unknown");
    }

    #[test]
    fn flows_are_derived_from_configured_dependencies() {
        let mut config = base_config();
        config.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![
                ServiceRegistryService {
                    name: "collector".to_string(),
                    runtime: "container".to_string(),
                    selector: None,
                    recovery: None,
                    checks: vec![],
                    depends_on: vec![ServiceDependencySpec {
                        kind: "http".to_string(),
                        target: Some("broker".to_string()),
                    }],
                },
                ServiceRegistryService {
                    name: "broker".to_string(),
                    runtime: "container".to_string(),
                    selector: None,
                    recovery: None,
                    checks: vec![],
                    depends_on: vec![],
                },
            ],
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        assert_eq!(snapshot.flows.len(), 1);
        assert_eq!(snapshot.flows[0].edges.len(), 1);
        assert_eq!(snapshot.flows[0].edges[0].from_module_id, "collector");
        assert_eq!(snapshot.flows[0].edges[0].to_module_id, "broker");
    }

    #[test]
    fn json_serialization_includes_dgraph_sections() {
        let mut config = base_config();
        config.service_registry = Some(ServiceRegistryConfig {
            enabled: true,
            services: vec![ServiceRegistryService {
                name: "local-http-ctr".to_string(),
                runtime: "container".to_string(),
                selector: None,
                recovery: None,
                checks: vec![],
                depends_on: vec![],
            }],
        });
        config.desired_state = Some(DesiredStateConfig {
            enabled: true,
            services: vec![DesiredServiceSpec {
                name: "local-http-ctr".to_string(),
                desired: "running".to_string(),
            }],
        });

        let snapshot = build_local_dgraph_snapshot(&config);
        let value = serde_json::to_value(&snapshot).expect("snapshot should serialize");

        assert!(value.get("runtimes").is_some());
        assert!(value.get("modules").is_some());
        assert!(value.get("flows").is_some());
        assert!(value.get("health").is_some());
        assert!(value.get("capabilities").is_some());
    }

    #[test]
    fn source_guard_confirms_no_execution_or_config_write_patterns() {
        let source = include_str!("builder.rs");
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
        ] {
            assert!(
                !implementation.contains(forbidden),
                "dgraph snapshot builder must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
