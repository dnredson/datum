use chrono::Utc;
use datum::agent::discovery::{
    derive_capacity_profile_id, derive_host_incarnation_id, BootstrapMode, CapabilityCategory,
    CapabilityObservation, CapabilitySource, CpuObservation, FilesystemObservation,
    HostIdentityBasis, IdentityObservation, MemoryObservation, PlatformObservation,
    ResourceObservation, SmartSentinelDiscovery, SwapObservation, DISCOVERY_SCHEMA_VERSION,
};

fn valid_resources() -> ResourceObservation {
    let mut resources = ResourceObservation {
        capacity_profile_id: String::new(),
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
    };
    resources.capacity_profile_id = derive_capacity_profile_id(&resources);
    resources
}

fn valid_discovery() -> SmartSentinelDiscovery {
    SmartSentinelDiscovery {
        schema_version: DISCOVERY_SCHEMA_VERSION.to_string(),
        observed_at_utc: Utc::now(),
        node_id: "fog-01".to_string(),
        instance_id: "fog-instance-1".to_string(),
        host_id: "host:fog-01".to_string(),
        stage: Some("fog".to_string()),
        bootstrap_mode: BootstrapMode::Discovery,
        identity: IdentityObservation {
            hostname: "Xubuntu".to_string(),
            sentinel_version: "0.1.0".to_string(),
            host_incarnation_id: derive_host_incarnation_id(
                HostIdentityBasis::SystemUuid,
                "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            ),
            host_identity_basis: HostIdentityBasis::SystemUuid,
            host_identity_attested: false,
        },
        platform: PlatformObservation {
            os_family: "linux".to_string(),
            os_name: "Ubuntu".to_string(),
            os_version: "25.10".to_string(),
            arch: "x86_64".to_string(),
        },
        resources: valid_resources(),
        capabilities: vec![
            CapabilityObservation {
                name: "python".to_string(),
                category: CapabilityCategory::LanguageRuntime,
                present: true,
                available: true,
                version: Some("3.13.0".to_string()),
                source: CapabilitySource::Observed,
                detail: None,
            },
            CapabilityObservation {
                name: "docker".to_string(),
                category: CapabilityCategory::ContainerRuntime,
                present: true,
                available: false,
                version: Some("28.0.0".to_string()),
                source: CapabilitySource::Observed,
                detail: Some("daemon unavailable".to_string()),
            },
            CapabilityObservation {
                name: "wasmtime".to_string(),
                category: CapabilityCategory::ExecutionRuntime,
                present: true,
                available: true,
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
    assert_eq!(json["node_id"], "fog-01");
    assert_eq!(json["instance_id"], "fog-instance-1");
    assert_eq!(json["stage"], "fog");
    assert_eq!(json["identity"]["host_identity_basis"], "system_uuid");
    assert_eq!(json["identity"]["host_identity_attested"], false);
    assert_eq!(json["capabilities"][1]["name"], "docker");
    assert_eq!(json["capabilities"][1]["present"], true);
    assert_eq!(json["capabilities"][1]["available"], false);
    assert_eq!(json["safe_to_auto_execute"], false);
    assert_eq!(json["side_effects_performed"], false);
}

#[test]
fn discovery_contract_rejects_mutating_or_impossible_evidence() {
    let mut mutating = valid_discovery();
    mutating.side_effects_performed = true;
    assert!(mutating.validate().is_err());

    let mut authorizing = valid_discovery();
    authorizing.safe_to_auto_execute = true;
    assert!(authorizing.validate().is_err());

    let mut impossible_memory = valid_discovery();
    impossible_memory.resources.memory.used_bytes =
        impossible_memory.resources.memory.total_bytes + 1;
    assert!(impossible_memory.validate().is_err());

    let mut no_cpu = valid_discovery();
    no_cpu.resources.cpu.logical_cores = 0;
    assert!(no_cpu.validate().is_err());

    let mut impossible_capability = valid_discovery();
    impossible_capability.capabilities[0].present = false;
    impossible_capability.capabilities[0].available = true;
    assert!(impossible_capability.validate().is_err());

    let mut unnamed_capability = valid_discovery();
    unnamed_capability.capabilities[0].name.clear();
    assert!(unnamed_capability.validate().is_err());

    let mut attested = valid_discovery();
    attested.identity.host_identity_attested = true;
    assert!(attested.validate().is_err());
}

#[test]
fn host_incarnation_changes_when_machine_anchor_changes() {
    let old_host = derive_host_incarnation_id(HostIdentityBasis::SystemUuid, "machine-a");
    let same_host = derive_host_incarnation_id(HostIdentityBasis::SystemUuid, "machine-a");
    let replacement = derive_host_incarnation_id(HostIdentityBasis::SystemUuid, "machine-b");

    assert_eq!(old_host, same_host);
    assert_ne!(old_host, replacement);
}

#[test]
fn capacity_profile_ignores_usage_but_changes_with_capacity() {
    let original = valid_resources();

    let mut different_usage = original.clone();
    different_usage.cpu.usage_percent = Some(99.0);
    different_usage.memory.used_bytes = 7 * 1024 * 1024 * 1024;
    different_usage.memory.available_bytes = 1024 * 1024 * 1024;
    different_usage.filesystems[0].used_bytes = 40 * 1024 * 1024 * 1024;
    different_usage.filesystems[0].available_bytes = 10 * 1024 * 1024 * 1024;
    assert_eq!(
        derive_capacity_profile_id(&original),
        derive_capacity_profile_id(&different_usage)
    );

    let mut upgraded_ram = original.clone();
    upgraded_ram.memory.total_bytes = 16 * 1024 * 1024 * 1024;
    upgraded_ram.memory.used_bytes = 2 * 1024 * 1024 * 1024;
    upgraded_ram.memory.available_bytes = 14 * 1024 * 1024 * 1024;
    assert_ne!(
        derive_capacity_profile_id(&original),
        derive_capacity_profile_id(&upgraded_ram)
    );
}
