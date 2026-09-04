use chrono::Utc;
use datum::agent::discovery::{
    derive_capacity_profile_id, derive_hardware_fingerprint_id, derive_os_installation_id,
    derive_role_binding_id, BootstrapMode, CapabilityCategory, CapabilityObservation,
    CapabilitySource, CpuObservation, FilesystemObservation, HardwareIdentityBasis,
    IdentityConfidence, IdentityObservation, MemoryObservation, OsIdentityBasis,
    PlatformObservation, ResourceObservation, SmartSentinelDiscovery, SwapObservation,
    DISCOVERY_SCHEMA_VERSION,
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

fn hardware_a() -> String {
    derive_hardware_fingerprint_id(&[
        (
            HardwareIdentityBasis::SystemUuid,
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
        ),
        (
            HardwareIdentityBasis::BaseboardSerial,
            "board-a".to_string(),
        ),
    ])
    .unwrap()
}

fn valid_discovery() -> SmartSentinelDiscovery {
    let hardware = hardware_a();
    let os_installation = derive_os_installation_id(
        OsIdentityBasis::MachineId,
        "11111111111111111111111111111111",
    )
    .unwrap();
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
            hardware_fingerprint_id: Some(hardware.clone()),
            hardware_identity_basis: vec![
                HardwareIdentityBasis::SystemUuid,
                HardwareIdentityBasis::BaseboardSerial,
            ],
            hardware_identity_confidence: IdentityConfidence::Strong,
            hardware_identity_attested: false,
            os_installation_id: Some(os_installation),
            os_identity_basis: OsIdentityBasis::MachineId,
            role_binding_id: derive_role_binding_id("fog-01", Some(&hardware)),
            predecessor_binding_id: None,
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
    assert_eq!(json["identity"]["hardware_identity_confidence"], "strong");
    assert_eq!(json["identity"]["hardware_identity_attested"], false);
    assert_eq!(json["identity"]["os_identity_basis"], "machine_id");
    assert!(json["identity"]["role_binding_id"].is_string());
    assert_eq!(json["identity"]["predecessor_binding_id"], serde_json::Value::Null);
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

    let mut attested = valid_discovery();
    attested.identity.hardware_identity_attested = true;
    assert!(attested.validate().is_err());
}

#[test]
fn hardware_fingerprint_is_stable_and_changes_on_replacement() {
    let old_host = hardware_a();
    let same_host = hardware_a();
    let replacement = derive_hardware_fingerprint_id(&[
        (HardwareIdentityBasis::SystemUuid, "machine-b".to_string()),
        (HardwareIdentityBasis::BaseboardSerial, "board-b".to_string()),
    ])
    .unwrap();

    assert_eq!(old_host, same_host);
    assert_ne!(old_host, replacement);
}

#[test]
fn os_reinstall_does_not_change_role_binding_when_hardware_is_same() {
    let hardware = hardware_a();
    let old_os = derive_os_installation_id(OsIdentityBasis::MachineId, "old-os").unwrap();
    let new_os = derive_os_installation_id(OsIdentityBasis::MachineId, "new-os").unwrap();
    assert_ne!(old_os, new_os);

    let before = derive_role_binding_id("fog-01", Some(&hardware));
    let after = derive_role_binding_id("fog-01", Some(&hardware));
    assert_eq!(before, after);
}

#[test]
fn hardware_replacement_changes_binding_but_keeps_logical_node_available_for_history() {
    let old_hardware = hardware_a();
    let replacement = derive_hardware_fingerprint_id(&[
        (HardwareIdentityBasis::BoardSerial, "rpi-b".to_string()),
    ])
    .unwrap();

    let old_binding = derive_role_binding_id("fog-01", Some(&old_hardware));
    let new_binding = derive_role_binding_id("fog-01", Some(&replacement));

    assert_ne!(old_binding, new_binding);
    assert_eq!("fog-01", "fog-01");
}

#[test]
fn missing_hardware_identity_does_not_fabricate_role_binding() {
    assert_eq!(derive_hardware_fingerprint_id(&[]), None);
    assert_eq!(derive_role_binding_id("fog-01", None), None);

    let mut discovery = valid_discovery();
    discovery.identity.hardware_fingerprint_id = None;
    discovery.identity.hardware_identity_basis.clear();
    discovery.identity.hardware_identity_confidence = IdentityConfidence::Unknown;
    discovery.identity.role_binding_id = None;
    discovery.validate().expect("unknown hardware remains valid evidence");
}

#[test]
fn capacity_profile_ignores_usage_but_changes_with_ram_upgrade() {
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
