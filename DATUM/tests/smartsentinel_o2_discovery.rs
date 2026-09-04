use chrono::Utc;
use datum::agent::discovery::{
    BootstrapMode, CapabilityCategory, CapabilityObservation, CapabilitySource, CpuObservation,
    FilesystemObservation, IdentityObservation, MemoryObservation, PlatformObservation,
    ResourceObservation, SmartSentinelDiscovery, SwapObservation, DISCOVERY_SCHEMA_VERSION,
};

fn valid_discovery() -> SmartSentinelDiscovery {
    SmartSentinelDiscovery {
        schema_version: DISCOVERY_SCHEMA_VERSION.to_string(),
        observed_at_utc: Utc::now(),
        node_id: "fog-01".to_string(),
        host_id: "host:fog-01".to_string(),
        bootstrap_mode: BootstrapMode::Discovery,
        identity: IdentityObservation {
            hostname: "Xubuntu".to_string(),
            sentinel_version: "0.1.0".to_string(),
        },
        platform: PlatformObservation {
            os_family: "linux".to_string(),
            os_name: "Ubuntu".to_string(),
            os_version: "25.10".to_string(),
            arch: "x86_64".to_string(),
        },
        resources: ResourceObservation {
            uptime_seconds: Some(1234),
            cpu: CpuObservation {
                logical_cores: 4,
                usage_percent: Some(12.5),
            },
            memory: MemoryObservation {
                total_bytes: 8 * 1024 * 1024 * 1024,
                used_bytes: 2 * 1024 * 1024 * 1024,
                available_bytes: 6 * 1024 * 1024 * 1024,
            },
            swap: SwapObservation {
                total_bytes: 2 * 1024 * 1024 * 1024,
                used_bytes: 0,
                available_bytes: 2 * 1024 * 1024 * 1024,
            },
            filesystems: vec![FilesystemObservation {
                mount: "/".to_string(),
                total_bytes: 50 * 1024 * 1024 * 1024,
                used_bytes: 25 * 1024 * 1024 * 1024,
                available_bytes: 25 * 1024 * 1024 * 1024,
            }],
        },
        capabilities: vec![
            CapabilityObservation {
                name: "python".to_string(),
                category: CapabilityCategory::LanguageRuntime,
                present: true,
                usable: true,
                version: Some("3.13.0".to_string()),
                source: CapabilitySource::Observed,
                detail: None,
            },
            CapabilityObservation {
                name: "docker".to_string(),
                category: CapabilityCategory::ContainerRuntime,
                present: true,
                usable: false,
                version: Some("28.0.0".to_string()),
                source: CapabilitySource::Observed,
                detail: Some("daemon unavailable".to_string()),
            },
            CapabilityObservation {
                name: "wasm".to_string(),
                category: CapabilityCategory::ExecutionRuntime,
                present: true,
                usable: true,
                version: None,
                source: CapabilitySource::Embedded,
                detail: Some("embedded wasmtime execution path".to_string()),
            },
        ],
        safe_to_auto_execute: false,
        side_effects_performed: false,
    }
}

#[test]
fn typed_discovery_contract_accepts_observation_only_report() {
    let discovery = valid_discovery();
    discovery.validate().expect("valid discovery contract");

    let json = serde_json::to_value(&discovery).expect("serialize discovery");
    assert_eq!(json["schema_version"], DISCOVERY_SCHEMA_VERSION);
    assert_eq!(json["bootstrap_mode"], "discovery");
    assert_eq!(json["capabilities"][1]["name"], "docker");
    assert_eq!(json["capabilities"][1]["present"], true);
    assert_eq!(json["capabilities"][1]["usable"], false);
    assert_eq!(json["side_effects_performed"], false);
}

#[test]
fn discovery_contract_rejects_mutating_or_impossible_evidence() {
    let mut mutating = valid_discovery();
    mutating.side_effects_performed = true;
    assert!(mutating.validate().is_err());

    let mut impossible_memory = valid_discovery();
    impossible_memory.resources.memory.used_bytes =
        impossible_memory.resources.memory.total_bytes + 1;
    assert!(impossible_memory.validate().is_err());

    let mut no_cpu = valid_discovery();
    no_cpu.resources.cpu.logical_cores = 0;
    assert!(no_cpu.validate().is_err());

    let mut impossible_capability = valid_discovery();
    impossible_capability.capabilities[0].present = false;
    impossible_capability.capabilities[0].usable = true;
    assert!(impossible_capability.validate().is_err());

    let mut unnamed_capability = valid_discovery();
    unnamed_capability.capabilities[0].name.clear();
    assert!(unnamed_capability.validate().is_err());
}
