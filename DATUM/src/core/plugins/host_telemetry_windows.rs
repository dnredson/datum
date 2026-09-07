use anyhow::Result;
use serde_json::{json, Value as JsonValue};
use std::process::Command;

use crate::core::model::Snapshot;
use crate::core::plugin::Plugin;

pub struct HostTelemetryWindowsPlugin;

impl HostTelemetryWindowsPlugin {
    pub fn new() -> Self {
        Self
    }
}

impl Plugin for HostTelemetryWindowsPlugin {
    fn name(&self) -> &'static str {
        "host_telemetry_windows"
    }

    fn apply(&self, snapshot: &mut Snapshot, _cfg: &crate::config::Config) -> Result<()> {
        if !cfg!(windows) {
            return Ok(());
        }

        let host_id = snapshot.meta.host_id.clone();
        let host_ent = match snapshot.entities.iter_mut().find(|e| e.id == host_id) {
            Some(e) => e,
            None => return Ok(()),
        };

        // Inclui CPU total via Win32_PerfFormattedData_PerfOS_Processor (Name='_Total')
        let ps = r#"
$os = Get-CimInstance Win32_OperatingSystem;
$cs = Get-CimInstance Win32_ComputerSystem;
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1;
$perf = Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'";
$disks = @(Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | Select-Object DeviceID, Size, FreeSpace);
$swap = @(Get-CimInstance Win32_PageFileUsage);
$interfaces = @(Get-NetAdapter | ForEach-Object {
  $adapter = $_;
  $stats = Get-NetAdapterStatistics -Name $adapter.Name -ErrorAction SilentlyContinue;
  [pscustomobject]@{
    name = $adapter.Name
    up = ($adapter.Status -eq "Up")
    addresses = @(Get-NetIPAddress -InterfaceIndex $adapter.ifIndex -ErrorAction SilentlyContinue | Select-Object -ExpandProperty IPAddress)
    rx_bytes = if ($stats) { [UInt64]$stats.ReceivedBytes } else { $null }
    tx_bytes = if ($stats) { [UInt64]$stats.SentBytes } else { $null }
    rx_errors = if ($stats) { [UInt64]$stats.InReceivedErrors } else { $null }
    tx_errors = if ($stats) { [UInt64]$stats.OutErrors } else { $null }
  }
});

$mem_total_bytes = [UInt64]$cs.TotalPhysicalMemory;
$mem_available_bytes = [UInt64]($os.FreePhysicalMemory * 1KB);
$mem_used_bytes = $mem_total_bytes - $mem_available_bytes;
$swap_total_bytes = [UInt64]((($swap | Measure-Object -Property AllocatedBaseSize -Sum).Sum) * 1MB);
$swap_used_bytes = [UInt64]((($swap | Measure-Object -Property CurrentUsage -Sum).Sum) * 1MB);
$filesystems = @($disks | ForEach-Object {
  [pscustomobject]@{
    mount = $_.DeviceID
    total_bytes = [UInt64]$_.Size
    used_bytes = [UInt64]($_.Size - $_.FreeSpace)
    available_bytes = [UInt64]$_.FreeSpace
  }
});

[pscustomobject]@{
  os_caption = $os.Caption
  os_version = $os.Version
  os_build   = $os.BuildNumber
  kernel = $os.Version
  arch_bits  = $cpu.AddressWidth
  cpu_logical = $cpu.NumberOfLogicalProcessors
  cpu_usage_pct = $perf.PercentProcessorTime
  uptime_seconds = [UInt64]((Get-Date) - $os.LastBootUpTime).TotalSeconds
  mem_total_mb = [math]::Round($mem_total_bytes / 1MB)
  mem_available_mb  = [math]::Round($mem_available_bytes / 1MB)
  mem_used_mb  = [math]::Round($mem_used_bytes / 1MB)
  mem_total_bytes = $mem_total_bytes
  mem_available_bytes = $mem_available_bytes
  mem_used_bytes = $mem_used_bytes
  swap_total_bytes = $swap_total_bytes
  swap_used_bytes = $swap_used_bytes
  process_count = @(Get-Process).Count
  interfaces = $interfaces
  filesystems = $filesystems
} | ConvertTo-Json -Compress
"#;

        let out = Command::new("powershell")
            .args(["-NoProfile", "-Command", ps])
            .output();

        let stdout = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Ok(o) => {
                eprintln!(
                    "plugin={} status=skipped err={}",
                    self.name(),
                    String::from_utf8_lossy(&o.stderr).trim()
                );
                return Ok(());
            }
            Err(e) => {
                eprintln!("plugin={} status=skipped err={}", self.name(), e);
                return Ok(());
            }
        };

        let system_json: JsonValue = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("plugin={} status=warn err=parse_json {}", self.name(), e);
                return Ok(());
            }
        };

        if !host_ent.attrs.is_object() {
            host_ent.attrs = json!({});
        }
        let obj = host_ent.attrs.as_object_mut().unwrap();
        obj.insert("system".to_string(), system_json);

        Ok(())
    }
}
