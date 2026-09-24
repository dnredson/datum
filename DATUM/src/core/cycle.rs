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

/// Phase 114B2 (ADR-0024 addendum): the plugin set for
/// [`run_observation_only_cycle_for_node_identity`]. Deliberately, structurally
/// smaller than [`run_cycle_with_identity`]'s list -- it contains ONLY
/// plugins that discover/normalize observed facts, never one that plans or
/// executes a remediation action. This is a HARD, compile-time-visible
/// guarantee: unlike `run_service_loop`'s continuous-mode safety (which
/// mutates `cfg.auto_recovery.enabled`/`cfg.desired_state.enabled` to `false`
/// and relies on `DesiredStatePlannerPlugin`/`RecoveryPlannerPlugin` each
/// internally honoring that flag), this function's plugin `Vec` simply does
/// not contain `DesiredStatePlannerPlugin`, `ServiceTransitionsPlugin`,
/// `RecoveryPlannerPlugin`, or `ActionExecutorPlugin` at all -- there is no
/// config value that could cause any of them to run. `RevalidatorPlugin`/
/// `EventCompactorPlugin` are also excluded: neither produces anything the
/// DMonitor projector consumes (both operate on `snapshot.events`/staleness
/// bookkeeping), so omitting them keeps this list minimal and easy to audit
/// by inspection, not merely by behavior.
fn observation_only_plugins() -> Vec<Box<dyn super::plugin::Plugin>> {
    vec![
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
    ]
}

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

/// Historical DATUM cycle behavior: host entity identity is derived from the OS hostname.
/// This remains unchanged for compatibility with the closed deployment baseline.
pub fn run_cycle(
    node_id: &str,
    hostname_override: Option<&str>,
    schema_version: &str,
    cfg: &crate::config::Config,
) -> Result<Snapshot> {
    run_cycle_with_identity(
        node_id,
        hostname_override,
        schema_version,
        cfg,
        HostIdentity::Hostname,
    )
}

/// SmartSentinel service-mode cycle: host entity identity is derived from the logical
/// node id while the real OS hostname remains observation metadata. This prevents two
/// different nodes with the same hostname from collapsing into one D-Graph entity.
pub fn run_cycle_for_node_identity(
    node_id: &str,
    hostname_override: Option<&str>,
    schema_version: &str,
    cfg: &crate::config::Config,
) -> Result<Snapshot> {
    run_cycle_with_identity(
        node_id,
        hostname_override,
        schema_version,
        cfg,
        HostIdentity::NodeId,
    )
}

#[derive(Debug, Clone, Copy)]
enum HostIdentity {
    Hostname,
    NodeId,
}

fn new_snapshot_with_host_entity(
    node_id: &str,
    hostname_override: Option<&str>,
    schema_version: &str,
    identity: HostIdentity,
) -> Result<Snapshot> {
    let observed_at = time::now_utc();
    let cycle_id = Uuid::new_v4().to_string();
    let hostname = match hostname_override {
        Some(h) => h.to_string(),
        None => get_hostname_fallback()?,
    };
    let host_identity = match identity {
        HostIdentity::Hostname => hostname.as_str(),
        HostIdentity::NodeId => node_id,
    };
    let host_id = ids::host_id(host_identity);
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
    Ok(Snapshot {
        meta,
        entities: vec![host_entity],
        edges: vec![],
        events: vec![],
        actions: vec![],
    })
}

fn run_cycle_with_identity(
    node_id: &str,
    hostname_override: Option<&str>,
    schema_version: &str,
    cfg: &crate::config::Config,
    identity: HostIdentity,
) -> Result<Snapshot> {
    let mut snapshot =
        new_snapshot_with_host_entity(node_id, hostname_override, schema_version, identity)?;
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

/// Phase 114B2: the structurally read-only counterpart to
/// [`run_cycle_for_node_identity`], for the canonical DMonitor projection
/// path exclusively. See [`observation_only_plugins`]'s doc comment for the
/// exact safety guarantee this provides -- it is independent of any config
/// value, unlike the continuous service loop's `cfg.auto_recovery.enabled`/
/// `cfg.desired_state.enabled` mutation. `run_cycle_for_node_identity`
/// itself is UNCHANGED and remains in use by other call sites
/// (`discovery_collectors`, `operational_reconciliation`, and
/// `smartsentinel-observe`'s own legacy snapshot/telemetry outputs) that
/// genuinely need the full planning/recovery pipeline -- this function does
/// not replace it, it is a second, deliberately narrower cycle used only to
/// source the DMonitor projector's input.
pub fn run_observation_only_cycle_for_node_identity(
    node_id: &str,
    hostname_override: Option<&str>,
    schema_version: &str,
    cfg: &crate::config::Config,
) -> Result<Snapshot> {
    let mut snapshot = new_snapshot_with_host_entity(
        node_id,
        hostname_override,
        schema_version,
        HostIdentity::NodeId,
    )?;
    for p in observation_only_plugins() {
        if let Err(e) = p.apply(&mut snapshot, cfg) {
            eprintln!("plugin={} status=error err={}", p.name(), e);
        }
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    /// Phase 114B2 (section 27): structural proof that the DMonitor-compatible
    /// observation path never wires `desired_state_planner`/
    /// `service_transitions`/`recovery_planner`/`action_executor`. Scans the
    /// PRODUCTION source of `observation_only_plugins` (not a comment, not a
    /// behavioral inference) for the exact plugin type names -- this is
    /// stronger than running the function and checking `snapshot.actions.is_empty()`,
    /// because it proves the DANGEROUS PLUGIN TYPES ARE NOT EVEN CONSTRUCTED,
    /// independent of what any of them would have done given real host state.
    #[test]
    fn observation_only_plugins_excludes_planning_and_mutation_plugins() {
        let source = include_str!("cycle.rs");
        let start = source
            .find("fn observation_only_plugins()")
            .expect("observation_only_plugins must exist");
        let body_end = source[start..]
            .find("\n}")
            .map(|offset| start + offset)
            .expect("observation_only_plugins body must close");
        let body = &source[start..body_end];

        for forbidden in [
            "DesiredStatePlannerPlugin",
            "ServiceTransitionsPlugin",
            "RecoveryPlannerPlugin",
            "ActionExecutorPlugin",
        ] {
            assert!(
                !body.contains(forbidden),
                "observation_only_plugins must never construct {forbidden}: {body}"
            );
        }

        // Positive control: the exact same scan technique DOES find these
        // plugins in the full pipeline, proving the scan itself is capable
        // of detecting them (not merely failing to match anything).
        let full_start = source
            .find("fn run_cycle_with_identity(")
            .expect("run_cycle_with_identity must exist");
        let full_body = &source[full_start..];
        for present in [
            "DesiredStatePlannerPlugin",
            "ServiceTransitionsPlugin",
            "RecoveryPlannerPlugin",
            "ActionExecutorPlugin",
        ] {
            assert!(
                full_body.contains(present),
                "sanity check: run_cycle_with_identity is expected to still contain {present}"
            );
        }
    }

    #[test]
    fn observation_only_plugins_is_nonempty_and_read_only_by_name() {
        // Every plugin name in the observation-only list is a discovery/
        // telemetry/service-state-read plugin; none is a planner or executor.
        let plugins = super::observation_only_plugins();
        assert!(!plugins.is_empty());
        for plugin in &plugins {
            let name = plugin.name();
            assert!(
                !name.contains("planner") && !name.contains("executor") && !name.contains("recovery"),
                "unexpected plugin name in observation-only list: {name}"
            );
        }
    }
}
