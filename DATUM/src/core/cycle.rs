use anyhow::{Context, Result};
use serde_json::json;
use uuid::Uuid;

use super::ids;
use super::model::{Entity, Meta, Snapshot};
use super::time;

use super::plugins::{
    action_executor::ActionExecutorPlugin, connection_ping_linux::ConnectionPingLinuxPlugin,
    connection_ping_windows::ConnectionPingWindowsPlugin,
    desired_state_planner::DesiredStatePlannerPlugin, docker_linux_cli::DockerLinuxCliPlugin,
    docker_windows_cli::DockerWindowsCliPlugin, event_compactor::EventCompactorPlugin,
    host_telemetry_linux::HostTelemetryLinuxPlugin,
    host_telemetry_windows::HostTelemetryWindowsPlugin, process_linux_ps::ProcessLinuxPsPlugin,
    process_windows_tasklist::ProcessWindowsTasklistPlugin,
    recovery_planner::RecoveryPlannerPlugin, revalidator::RevalidatorPlugin,
    service_builder::ServiceBuilderPlugin, service_health::ServiceHealthPlugin,
    service_transitions::ServiceTransitionsPlugin,
};

fn get_hostname_fallback() -> Result<String> {
    let hn = std::env::var("HOSTNAME").ok();
    if let Some(h) = hn {
        if !h.trim().is_empty() {
            return Ok(h);
        }
    }
    let out = std::process::Command::new("hostname")
        .output()
        .context("failed to execute `hostname` command")?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        anyhow::bail!("hostname is empty and HOSTNAME env var not set");
    }
    Ok(s)
}

pub fn run_cycle(
    node_id: &str,
    hostname_override: Option<&str>,
    schema_version: &str,
    cfg: &crate::config::Config,
) -> Result<Snapshot> {
    let observed_at = time::now_utc();
    let cycle_id = Uuid::new_v4().to_string();
    let hostname = match hostname_override {
        Some(h) => h.to_string(),
        None => get_hostname_fallback()?,
    };
    let host_id = ids::host_id(&hostname);
    let host_entity = Entity {
        id: host_id.clone(),
        kind: "host".to_string(),
        attrs: json!({"hostname": hostname,"node_id": node_id}),
        observed_at_utc: observed_at,
        ttl_s: 60,
    };
    let meta = Meta {
        schema_version: schema_version.to_string(),
        agent_version: env!("CARGO_PKG_VERSION").to_string(),
        cycle_id,
        observed_at_utc: observed_at,
        node_id: node_id.to_string(),
        host_id,
    };
    let mut snapshot = Snapshot {
        meta,
        entities: vec![host_entity],
        edges: vec![],
        events: vec![],
        actions: vec![],
    };
    let plugins: Vec<Box<dyn super::plugin::Plugin>> = vec![
        Box::new(DockerWindowsCliPlugin::new()),
        Box::new(DockerLinuxCliPlugin::new()),
        Box::new(HostTelemetryWindowsPlugin::new()),
        Box::new(HostTelemetryLinuxPlugin::new()),
        Box::new(ProcessWindowsTasklistPlugin::new()),
        Box::new(ProcessLinuxPsPlugin::new()),
        Box::new(ConnectionPingWindowsPlugin::new()),
        Box::new(ConnectionPingLinuxPlugin::new()),
        Box::new(ServiceBuilderPlugin::new()),
        Box::new(ServiceHealthPlugin::new()),
        Box::new(DesiredStatePlannerPlugin::new()),
        Box::new(ServiceTransitionsPlugin::new()),
        Box::new(RecoveryPlannerPlugin::new()),
        Box::new(ActionExecutorPlugin::new()),
        Box::new(RevalidatorPlugin::new()),
        Box::new(EventCompactorPlugin::new()),
    ];
    for p in plugins {
        if let Err(e) = p.apply(&mut snapshot, cfg) {
            eprintln!("plugin={} status=error err={}", p.name(), e);
        }
    }
    Ok(snapshot)
}
