use std::fs;
use std::io::ErrorKind;
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::Value;

use crate::agent::discovery::{
    derive_capacity_profile_id, derive_hardware_fingerprint_id, derive_os_installation_id,
    derive_role_binding_id, BootstrapMode, CapabilityCategory, CapabilityObservation,
    CapabilitySource, CpuObservation, FilesystemObservation, HardwareIdentityBasis,
    IdentityConfidence, IdentityObservation, MemoryObservation, OsIdentityBasis,
    PlatformObservation, ResourceObservation, SmartSentinelDiscovery, SwapObservation,
    DISCOVERY_SCHEMA_VERSION,
};
use crate::config::Config;
use crate::core::cycle::run_cycle_for_node_identity;

pub fn collect_discovery(
    node_id: &str,
    hostname_override: Option<&str>,
    bootstrap_mode: BootstrapMode,
    cfg: &Config,
) -> Result<SmartSentinelDiscovery> {
    if node_id.trim().is_empty() {
        bail!("SmartSentinel discovery node_id must not be empty");
    }

    let mut safe_cfg = cfg.clone();
    if let Some(auto_recovery) = safe_cfg.auto_recovery.as_mut() {
        auto_recovery.enabled = false;
        auto_recovery.dry_run = Some(true);
    }
    if let Some(desired_state) = safe_cfg.desired_state.as_mut() {
        desired_state.enabled = false;
    }

    let snapshot = run_cycle_for_node_identity(
        node_id,
        hostname_override,
        &safe_cfg.agent.schema_version,
        &safe_cfg,
    )?;

    let host = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == snapshot.meta.host_id)
        .context("SmartSentinel discovery host entity missing from observation cycle")?;

    let hostname = host
        .attrs
        .get("hostname")
        .and_then(Value::as_str)
        .unwrap_or(node_id)
        .to_string();
    let system = host.attrs.get("system").cloned().unwrap_or(Value::Null);

    let (platform, mut resources) = typed_host_from_system(&system)?;
    supplement_host_resources(&mut resources)?;
    resources.capacity_profile_id = derive_capacity_profile_id(&resources);

    let (hardware_fingerprint_id, hardware_identity_basis, hardware_identity_confidence) =
        collect_hardware_identity();
    let (os_installation_id, os_identity_basis) = collect_os_identity();
    let role_binding_id = derive_role_binding_id(node_id, hardware_fingerprint_id.as_deref());

    let discovery = SmartSentinelDiscovery {
        schema_version: DISCOVERY_SCHEMA_VERSION.to_string(),
        observed_at_utc: Utc::now(),
        node_id: node_id.to_string(),
        instance_id: uuid::Uuid::new_v4().to_string(),
        host_id: snapshot.meta.host_id,
        stage: safe_cfg.agent.stage.clone(),
        bootstrap_mode,
        identity: IdentityObservation {
            hostname,
            sentinel_version: env!("CARGO_PKG_VERSION").to_string(),
            hardware_fingerprint_id,
            hardware_identity_basis,
            hardware_identity_confidence,
            hardware_identity_attested: false,
            os_installation_id,
            os_identity_basis,
            role_binding_id,
            predecessor_binding_id: None,
        },
        platform,
        resources,
        capabilities: collect_capabilities(),
        safe_to_auto_execute: false,
        side_effects_performed: false,
    };

    discovery.validate()?;
    Ok(discovery)
}

fn typed_host_from_system(system: &Value) -> Result<(PlatformObservation, ResourceObservation)> {
    if cfg!(windows) {
        typed_windows_host(system)
    } else {
        typed_linux_host(system)
    }
}

fn typed_linux_host(system: &Value) -> Result<(PlatformObservation, ResourceObservation)> {
    let logical_cores = usize_field(system, "cpu_logical")?;
    let memory = memory_from_mb_fields(system)?;
    let filesystem = FilesystemObservation {
        mount: "/".to_string(),
        total_bytes: gib_field_to_bytes(system, "disk_root_total_gb"),
        used_bytes: gib_field_to_bytes(system, "disk_root_used_gb"),
        available_bytes: gib_field_to_bytes(system, "disk_root_free_gb"),
    };

    Ok((
        PlatformObservation {
            os_family: "linux".to_string(),
            os_name: string_field(system, "os_name").unwrap_or_else(|| "Linux".to_string()),
            os_version: string_field(system, "os_version").unwrap_or_default(),
            arch: string_field(system, "arch").unwrap_or_else(|| std::env::consts::ARCH.to_string()),
        },
        ResourceObservation {
            capacity_profile_id: String::new(),
            uptime_seconds: None,
            cpu: CpuObservation {
                logical_cores,
                usage_percent: float_field(system, "cpu_usage_pct"),
            },
            memory,
            swap: SwapObservation {
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
            },
            filesystems: vec![filesystem],
        },
    ))
}

fn typed_windows_host(system: &Value) -> Result<(PlatformObservation, ResourceObservation)> {
    let logical_cores = usize_field(system, "cpu_logical")?;
    let memory = memory_from_mb_fields(system)?;
    let mut filesystems = Vec::new();

    if let Some(disks) = system.get("disks") {
        for disk in array_or_singleton(disks) {
            let total = disk.get("Size").and_then(value_as_u64).unwrap_or(0);
            let available = disk.get("FreeSpace").and_then(value_as_u64).unwrap_or(0);
            let mount = disk
                .get("DeviceID")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            filesystems.push(FilesystemObservation {
                mount,
                total_bytes: total,
                used_bytes: total.saturating_sub(available),
                available_bytes: available,
            });
        }
    }

    if filesystems.is_empty() {
        filesystems.push(FilesystemObservation {
            mount: "unknown".to_string(),
            total_bytes: 0,
            used_bytes: 0,
            available_bytes: 0,
        });
    }

    Ok((
        PlatformObservation {
            os_family: "windows".to_string(),
            os_name: string_field(system, "os_caption").unwrap_or_else(|| "Windows".to_string()),
            os_version: string_field(system, "os_version").unwrap_or_default(),
            arch: std::env::consts::ARCH.to_string(),
        },
        ResourceObservation {
            capacity_profile_id: String::new(),
            uptime_seconds: None,
            cpu: CpuObservation {
                logical_cores,
                usage_percent: float_field(system, "cpu_usage_pct"),
            },
            memory,
            swap: SwapObservation {
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
            },
            filesystems,
        },
    ))
}

fn supplement_host_resources(resources: &mut ResourceObservation) -> Result<()> {
    if cfg!(windows) {
        supplement_windows_resources(resources)
    } else {
        supplement_linux_resources(resources)
    }
}

fn supplement_linux_resources(resources: &mut ResourceObservation) -> Result<()> {
    resources.uptime_seconds = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|raw| raw.split_whitespace().next()?.parse::<f64>().ok())
        .map(|seconds| seconds.max(0.0) as u64);

    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let swap_total_kb = meminfo_kb(&meminfo, "SwapTotal:").unwrap_or(0);
    let swap_free_kb = meminfo_kb(&meminfo, "SwapFree:").unwrap_or(0);
    resources.swap = SwapObservation {
        total_bytes: swap_total_kb.saturating_mul(1024),
        used_bytes: swap_total_kb.saturating_sub(swap_free_kb).saturating_mul(1024),
        available_bytes: swap_free_kb.saturating_mul(1024),
    };
    Ok(())
}

fn supplement_windows_resources(resources: &mut ResourceObservation) -> Result<()> {
    let script = r#"
$os = Get-CimInstance Win32_OperatingSystem;
$page = @(Get-CimInstance Win32_PageFileUsage);
$uptime = [math]::Floor(((Get-Date) - $os.LastBootUpTime).TotalSeconds);
$total = ($page | Measure-Object -Property AllocatedBaseSize -Sum).Sum;
$used = ($page | Measure-Object -Property CurrentUsage -Sum).Sum;
if ($null -eq $total) { $total = 0 }
if ($null -eq $used) { $used = 0 }
[pscustomobject]@{ uptime_seconds=$uptime; swap_total_mb=$total; swap_used_mb=$used } | ConvertTo-Json -Compress
"#;

    let out = Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .output();
    let Ok(out) = out else {
        return Ok(());
    };
    if !out.status.success() {
        return Ok(());
    }
    let value: Value = match serde_json::from_slice(&out.stdout) {
        Ok(value) => value,
        Err(_) => return Ok(()),
    };
    resources.uptime_seconds = value.get("uptime_seconds").and_then(value_as_u64);
    let total = value
        .get("swap_total_mb")
        .and_then(value_as_u64)
        .unwrap_or(0)
        .saturating_mul(1024 * 1024);
    let used = value
        .get("swap_used_mb")
        .and_then(value_as_u64)
        .unwrap_or(0)
        .saturating_mul(1024 * 1024);
    resources.swap = SwapObservation {
        total_bytes: total,
        used_bytes: used.min(total),
        available_bytes: total.saturating_sub(used.min(total)),
    };
    Ok(())
}

fn collect_hardware_identity() -> (
    Option<String>,
    Vec<HardwareIdentityBasis>,
    IdentityConfidence,
) {
    let mut anchors = Vec::<(HardwareIdentityBasis, String)>::new();

    if cfg!(windows) {
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::SystemUuid,
            windows_system_uuid(),
        );
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::BaseboardSerial,
            windows_baseboard_serial(),
        );
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::SystemSerial,
            windows_bios_serial(),
        );
    } else {
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::SystemUuid,
            read_meaningful_anchor("/sys/class/dmi/id/product_uuid"),
        );
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::BaseboardSerial,
            read_meaningful_anchor("/sys/class/dmi/id/board_serial"),
        );
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::SystemSerial,
            read_meaningful_anchor("/sys/class/dmi/id/product_serial"),
        );
        push_anchor(
            &mut anchors,
            HardwareIdentityBasis::BoardSerial,
            linux_board_serial(),
        );
    }

    if anchors.is_empty() {
        return (None, Vec::new(), IdentityConfidence::Unknown);
    }

    let confidence = match anchors.len() {
        1 => IdentityConfidence::Medium,
        _ => IdentityConfidence::Strong,
    };
    let basis = anchors.iter().map(|(basis, _)| *basis).collect::<Vec<_>>();
    (
        derive_hardware_fingerprint_id(&anchors),
        basis,
        confidence,
    )
}

fn collect_os_identity() -> (Option<String>, OsIdentityBasis) {
    if cfg!(windows) {
        if let Some(anchor) = windows_machine_guid() {
            return (
                derive_os_installation_id(OsIdentityBasis::MachineGuid, &anchor),
                OsIdentityBasis::MachineGuid,
            );
        }
    } else if let Some(anchor) = read_meaningful_anchor("/etc/machine-id") {
        return (
            derive_os_installation_id(OsIdentityBasis::MachineId, &anchor),
            OsIdentityBasis::MachineId,
        );
    }

    (None, OsIdentityBasis::Unknown)
}

fn push_anchor(
    anchors: &mut Vec<(HardwareIdentityBasis, String)>,
    basis: HardwareIdentityBasis,
    value: Option<String>,
) {
    if let Some(value) = value.filter(|value| meaningful_anchor(value)) {
        anchors.push((basis, value));
    }
}

fn windows_system_uuid() -> Option<String> {
    command_text(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystemProduct).UUID",
        ],
    )
}

fn windows_baseboard_serial() -> Option<String> {
    command_text(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_BaseBoard).SerialNumber",
        ],
    )
}

fn windows_bios_serial() -> Option<String> {
    command_text(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_BIOS).SerialNumber",
        ],
    )
}

fn windows_machine_guid() -> Option<String> {
    command_text(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-ItemProperty 'HKLM:\\SOFTWARE\\Microsoft\\Cryptography').MachineGuid",
        ],
    )
}

fn linux_board_serial() -> Option<String> {
    if let Some(value) = read_meaningful_anchor("/sys/firmware/devicetree/base/serial-number") {
        return Some(value);
    }
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    cpuinfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.trim().eq_ignore_ascii_case("serial") && meaningful_anchor(value) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn read_meaningful_anchor(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim_matches(char::from(0)).trim().to_string())
        .filter(|value| meaningful_anchor(value))
}

fn meaningful_anchor(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return false;
    }
    const PLACEHOLDERS: &[&str] = &[
        "unknown",
        "none",
        "not specified",
        "default string",
        "to be filled by o.e.m.",
        "system serial number",
    ];
    if PLACEHOLDERS.iter().any(|placeholder| normalized == *placeholder) {
        return false;
    }
    let compact = normalized
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    !compact.is_empty()
        && compact.chars().any(|ch| ch != '0')
        && compact.chars().any(|ch| ch != 'f')
}

fn command_text(name: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(name).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    nonempty_output(&out.stdout)
}

fn collect_capabilities() -> Vec<CapabilityObservation> {
    let mut capabilities = Vec::new();
    capabilities.push(probe_python());
    capabilities.push(probe_docker());
    capabilities.push(CapabilityObservation {
        name: "wasmtime".to_string(),
        category: CapabilityCategory::ExecutionRuntime,
        present: true,
        available: true,
        version: None,
        source: CapabilitySource::Embedded,
        detail: Some("embedded Wasmtime execution path compiled into DATUM".to_string()),
    });

    if cfg!(windows) {
        capabilities.push(probe_command(
            "powershell",
            &["-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"],
            CapabilityCategory::Tool,
        ));
    } else {
        let mut shell = probe_command("sh", &["-c", "exit 0"], CapabilityCategory::Tool);
        shell.version = None;
        capabilities.push(shell);
    }

    capabilities
}

fn probe_python() -> CapabilityObservation {
    let python3 = probe_command("python3", &["--version"], CapabilityCategory::LanguageRuntime);
    if python3.present {
        return CapabilityObservation {
            name: "python".to_string(),
            ..python3
        };
    }
    let python = probe_command("python", &["--version"], CapabilityCategory::LanguageRuntime);
    CapabilityObservation {
        name: "python".to_string(),
        ..python
    }
}

fn probe_docker() -> CapabilityObservation {
    let cli = probe_command("docker", &["--version"], CapabilityCategory::ContainerRuntime);
    if !cli.present {
        return CapabilityObservation {
            name: "docker".to_string(),
            ..cli
        };
    }

    let daemon = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output();
    match daemon {
        Ok(out) if out.status.success() => CapabilityObservation {
            name: "docker".to_string(),
            category: CapabilityCategory::ContainerRuntime,
            present: true,
            available: true,
            version: cli.version,
            source: CapabilitySource::Observed,
            detail: nonempty_output(&out.stdout),
        },
        Ok(out) => CapabilityObservation {
            name: "docker".to_string(),
            category: CapabilityCategory::ContainerRuntime,
            present: true,
            available: false,
            version: cli.version,
            source: CapabilitySource::Observed,
            detail: nonempty_output(&out.stderr)
                .or_else(|| Some("Docker CLI present but daemon/runtime unavailable".to_string())),
        },
        Err(err) => CapabilityObservation {
            name: "docker".to_string(),
            category: CapabilityCategory::ContainerRuntime,
            present: true,
            available: false,
            version: cli.version,
            source: CapabilitySource::Observed,
            detail: Some(err.to_string()),
        },
    }
}

fn probe_command(name: &str, args: &[&str], category: CapabilityCategory) -> CapabilityObservation {
    match Command::new(name).args(args).output() {
        Ok(out) => {
            let stdout = nonempty_output(&out.stdout);
            let stderr = nonempty_output(&out.stderr);
            CapabilityObservation {
                name: name.to_string(),
                category,
                present: true,
                available: out.status.success(),
                version: stdout.clone().or_else(|| stderr.clone()),
                source: CapabilitySource::Observed,
                detail: if out.status.success() { None } else { stderr.or(stdout) },
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => CapabilityObservation {
            name: name.to_string(),
            category,
            present: false,
            available: false,
            version: None,
            source: CapabilitySource::Observed,
            detail: Some("executable not found".to_string()),
        },
        Err(err) => CapabilityObservation {
            name: name.to_string(),
            category,
            present: true,
            available: false,
            version: None,
            source: CapabilitySource::Observed,
            detail: Some(err.to_string()),
        },
    }
}

fn memory_from_mb_fields(system: &Value) -> Result<MemoryObservation> {
    let total = mib_field_to_bytes(system, "mem_total_mb");
    let used = mib_field_to_bytes(system, "mem_used_mb");
    let available = mib_field_to_bytes(system, "mem_available_mb");
    if total == 0 {
        bail!("SmartSentinel discovery host collector did not report memory capacity");
    }
    Ok(MemoryObservation {
        total_bytes: total,
        used_bytes: used.min(total),
        available_bytes: available.min(total),
    })
}

fn usize_field(value: &Value, key: &str) -> Result<usize> {
    value
        .get(key)
        .and_then(value_as_u64)
        .map(|v| v as usize)
        .filter(|v| *v > 0)
        .with_context(|| format!("SmartSentinel discovery missing positive {}", key))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn float_field(value: &Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|n| n as f64)))
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| u64::try_from(v).ok()))
        .or_else(|| value.as_f64().filter(|v| *v >= 0.0).map(|v| v as u64))
}

fn mib_field_to_bytes(value: &Value, key: &str) -> u64 {
    float_field(value, key)
        .unwrap_or(0.0)
        .max(0.0)
        .mul_add((1024 * 1024) as f64, 0.0) as u64
}

fn gib_field_to_bytes(value: &Value, key: &str) -> u64 {
    float_field(value, key)
        .unwrap_or(0.0)
        .max(0.0)
        .mul_add((1024_u64 * 1024 * 1024) as f64, 0.0) as u64
}

fn array_or_singleton(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(values) => values.iter().collect(),
        Value::Null => Vec::new(),
        value => vec![value],
    }
}

fn meminfo_kb(raw: &str, field: &str) -> Option<u64> {
    raw.lines().find_map(|line| {
        if !line.starts_with(field) {
            return None;
        }
        line.split_whitespace()
            .find_map(|part| part.parse::<u64>().ok())
    })
}

fn nonempty_output(bytes: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(bytes).trim().to_string();
    (!value.is_empty()).then_some(value)
}
