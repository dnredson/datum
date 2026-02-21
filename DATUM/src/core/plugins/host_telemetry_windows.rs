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
$disks = Get-CimInstance Win32_LogicalDisk -Filter "DriveType=3" | Select-Object DeviceID, Size, FreeSpace;

$mem_total_mb = [math]::Round($cs.TotalPhysicalMemory / 1MB);
$mem_free_mb  = [math]::Round($os.FreePhysicalMemory / 1024);
$mem_used_mb  = $mem_total_mb - $mem_free_mb;

[pscustomobject]@{
  os_caption = $os.Caption
  os_version = $os.Version
  os_build   = $os.BuildNumber
  arch_bits  = $cpu.AddressWidth
  cpu_logical = $cpu.NumberOfLogicalProcessors
  cpu_usage_pct = $perf.PercentProcessorTime
  mem_total_mb = $mem_total_mb
  mem_available_mb  = $mem_free_mb
  mem_used_mb  = $mem_used_mb
  disks = $disks
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
