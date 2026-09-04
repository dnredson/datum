use std::collections::HashSet;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use wasmtime::{Config as WasmConfig, Engine, Instance, Module, Store};

use crate::agent::bootstrap::{run_bootstrap, BootstrapState};
use crate::config::{Config, DispatchContainerMapping};
use crate::core::cycle::run_cycle_for_node_identity;
use crate::outputs::http_json::HttpJsonOutput;

pub const OPERATIONAL_RECONCILER_MODULE_ID: &str = "smartsentinel-reconciler-v1";
pub const OPERATIONAL_RECONCILER_FUNCTION: &str = "reconcile_service";
const RECONCILER_WAT: &str = r#"
(module
  (func (export "reconcile_service") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.eqz
    i32.and))
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalDesiredService {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconciliationAuthorization {
    pub authorization_id: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub node_id: String,
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconcilerDescriptor {
    pub module_id: String,
    pub function_name: String,
    pub abi: String,
    pub host_imports_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraphSlice {
    pub schema_version: String,
    pub dgraph_id: String,
    pub project_id: String,
    pub node_id: String,
    pub desired_services: Vec<OperationalDesiredService>,
    #[serde(default)]
    pub authorization: Option<OperationalReconciliationAuthorization>,
    pub reconciler: OperationalReconcilerDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmReconciliationDecision {
    pub module_id: String,
    pub function_name: String,
    pub status: String,
    pub action_code: i32,
    pub host_imports_allowed: bool,
    pub guest_side_effects_performed: bool,
    pub fuel_configured: u64,
    pub fuel_consumed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalServiceReconciliationReport {
    pub service_id: String,
    pub desired_state: String,
    pub trusted_container_name: Option<String>,
    pub trusted_image: Option<String>,
    pub observed_before_running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_error: Option<String>,
    pub wasm: WasmReconciliationDecision,
    pub execution_status: String,
    pub message: String,
    pub observed_after_running: bool,
    pub host_mutation_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalReconciliationReport {
    pub bootstrap_state: BootstrapState,
    pub resource_id: String,
    pub node_id: String,
    pub dgraph_id: String,
    pub execute_requested: bool,
    pub control_plane_authorized: bool,
    pub local_container_execution_allowed: bool,
    pub services: Vec<OperationalServiceReconciliationReport>,
    pub side_effects_performed: bool,
    pub snapshot_published: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_publish_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerState {
    Running,
    Stopped,
    Missing,
}

pub fn run_operational_reconciliation(
    cfg: &Config,
    requested_node_id: Option<&str>,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
    execute_requested: bool,
) -> Result<OperationalReconciliationReport> {
    let bootstrap = run_bootstrap(
        cfg,
        requested_node_id,
        hostname_override,
        dserver_url,
        project_id,
        timeout_s,
    )?;
    if bootstrap.bootstrap_state != BootstrapState::KnownBoundNode {
        bail!(
            "operational reconciliation requires known_bound_node, received {:?}",
            bootstrap.bootstrap_state
        );
    }
    let node_id = bootstrap
        .control_plane
        .resolved_node_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("known_bound_node bootstrap did not resolve node_id"))?;

    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_s))
        .build()
        .context("build operational reconciliation HTTP client")?;
    let base = dserver_url.trim().trim_end_matches('/');
    let slice_url = format!("{base}/api/v1/nodes/{node_id}/operational-dgraph-slice");
    let response = client
        .get(&slice_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("fetch operational D-Graph slice {slice_url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!("operational D-Graph slice query failed with status {status}: {body}");
    }
    let slice: OperationalDGraphSlice = response
        .json()
        .context("decode operational D-Graph node slice")?;
    validate_slice(&slice, project_id, &node_id)?;

    let control_plane_authorized = slice.authorization.as_ref().is_some_and(|authorization| {
        authorization.allow_host_mutation
            && authorization.project_id == slice.project_id
            && authorization.dgraph_id == slice.dgraph_id
            && authorization.node_id == slice.node_id
    });
    let local_container_execution_allowed = cfg.dispatch.as_ref().is_some_and(|dispatch| {
        dispatch.enabled && dispatch.allow_container_execution.unwrap_or(false)
    });

    let mut side_effects_performed = false;
    let mut services = Vec::with_capacity(slice.desired_services.len());
    for service in &slice.desired_services {
        let mapping = trusted_container_mapping(cfg, &service.service_id);
        let (observed_before, observation_error) = match mapping {
            Some(mapping) => match inspect_container(&mapping.container_name) {
                Ok(state) => (state, None),
                Err(error) => (ContainerState::Missing, Some(error.to_string())),
            },
            None => (ContainerState::Missing, None),
        };
        let observed_before_running = observed_before == ContainerState::Running;
        let wasm = run_embedded_reconciler(true, observed_before_running)?;

        let (execution_status, message, host_mutation_performed) = if wasm.action_code == 0 {
            (
                "converged".to_string(),
                "desired service is already running; WASM requested no host action".to_string(),
                false,
            )
        } else if !execute_requested {
            (
                "suggested".to_string(),
                "WASM requested ensure_service_running, but --execute was not provided".to_string(),
                false,
            )
        } else if !control_plane_authorized {
            (
                "blocked_control_plane".to_string(),
                "WASM requested ensure_service_running, but the active D-Graph slice has no matching host-mutation authorization".to_string(),
                false,
            )
        } else if !local_container_execution_allowed {
            (
                "blocked_local_policy".to_string(),
                "WASM requested ensure_service_running, but local dispatch is disabled or allow_container_execution is not true".to_string(),
                false,
            )
        } else if mapping.is_none() {
            (
                "blocked_local_mapping".to_string(),
                format!(
                    "service '{}' is inside the authorized D-Graph slice, but no trusted local container mapping with image exists",
                    service.service_id
                ),
                false,
            )
        } else if let Some(error) = observation_error.as_deref() {
            (
                "docker_unavailable".to_string(),
                format!("WASM requested ensure_service_running, but Docker observation failed: {error}"),
                false,
            )
        } else {
            match execute_trusted_container_intent(cfg, &service.service_id, observed_before) {
                Ok(result) => result,
                Err(error) => (
                    "failed".to_string(),
                    format!("governed trusted container execution failed: {error}"),
                    false,
                ),
            }
        };

        side_effects_performed |= host_mutation_performed;
        let observed_after_running = mapping
            .and_then(|mapping| inspect_container(&mapping.container_name).ok())
            .is_some_and(|state| state == ContainerState::Running);

        services.push(OperationalServiceReconciliationReport {
            service_id: service.service_id.clone(),
            desired_state: service.desired_state.clone(),
            trusted_container_name: mapping.map(|mapping| mapping.container_name.clone()),
            trusted_image: mapping.and_then(|mapping| mapping.artifact.clone()),
            observed_before_running,
            observation_error,
            wasm,
            execution_status,
            message,
            observed_after_running,
            host_mutation_performed,
        });
    }

    let (snapshot_published, snapshot_publish_error) = match publish_role_snapshot(
        cfg,
        &node_id,
        hostname_override,
        base,
        project_id,
        timeout_s,
    ) {
        Ok(()) => (true, None),
        Err(error) => (false, Some(error.to_string())),
    };

    Ok(OperationalReconciliationReport {
        bootstrap_state: bootstrap.bootstrap_state,
        resource_id: bootstrap.control_plane.resource_id,
        node_id,
        dgraph_id: slice.dgraph_id,
        execute_requested,
        control_plane_authorized,
        local_container_execution_allowed,
        services,
        side_effects_performed,
        snapshot_published,
        snapshot_publish_error,
    })
}

pub fn run_embedded_reconciler(
    desired_running: bool,
    observed_running: bool,
) -> Result<WasmReconciliationDecision> {
    let bytes = wat::parse_str(RECONCILER_WAT).context("compile embedded reconciler WAT")?;
    let mut config = WasmConfig::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build operational reconciler Wasmtime engine")?;
    let module = Module::new(&engine, bytes).context("compile operational reconciler module")?;
    if module.imports().next().is_some() {
        bail!("operational reconciler must not import host functions");
    }

    let fuel_configured = 100_000u64;
    let mut store = Store::new(&engine, ());
    store
        .set_fuel(fuel_configured)
        .context("configure operational reconciler fuel")?;
    let fuel_before = store.get_fuel().ok();
    let instance = Instance::new(&mut store, &module, &[])
        .context("instantiate operational reconciler")?;
    let function = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, OPERATIONAL_RECONCILER_FUNCTION)
        .context("resolve operational reconciler function")?;
    let desired = if desired_running { 1 } else { 0 };
    let observed = if observed_running { 1 } else { 0 };
    let action_code = function
        .call(&mut store, (desired, observed))
        .context("execute operational reconciler WASM")?;
    if !matches!(action_code, 0 | 1) {
        bail!("operational reconciler returned unsupported action code {action_code}");
    }
    let fuel_after = store.get_fuel().ok();
    let fuel_consumed = match (fuel_before, fuel_after) {
        (Some(before), Some(after)) if before >= after => Some(before - after),
        _ => None,
    };

    Ok(WasmReconciliationDecision {
        module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
        function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
        status: "executed".to_string(),
        action_code,
        host_imports_allowed: false,
        guest_side_effects_performed: false,
        fuel_configured,
        fuel_consumed,
    })
}

fn validate_slice(slice: &OperationalDGraphSlice, project_id: &str, node_id: &str) -> Result<()> {
    if slice.project_id.trim() != project_id.trim() {
        bail!(
            "operational D-Graph slice project '{}' does not match requested project '{}'",
            slice.project_id,
            project_id
        );
    }
    if slice.node_id.trim() != node_id.trim() {
        bail!(
            "operational D-Graph slice node '{}' does not match resolved node '{}'",
            slice.node_id,
            node_id
        );
    }
    if slice.reconciler.module_id != OPERATIONAL_RECONCILER_MODULE_ID
        || slice.reconciler.function_name != OPERATIONAL_RECONCILER_FUNCTION
        || slice.reconciler.host_imports_allowed
    {
        bail!("operational D-Graph slice requests an unsupported reconciler contract");
    }

    let mut seen_services = HashSet::new();
    for service in &slice.desired_services {
        if service.service_id.trim().is_empty() {
            bail!("operational D-Graph desired service_id must not be empty");
        }
        if !seen_services.insert(service.service_id.trim().to_ascii_lowercase()) {
            bail!(
                "operational D-Graph slice contains duplicate desired service '{}'",
                service.service_id
            );
        }
        if !service.runtime.eq_ignore_ascii_case("container") {
            bail!("unsupported operational runtime '{}': expected container", service.runtime);
        }
        if !service.desired_state.eq_ignore_ascii_case("running") {
            bail!(
                "unsupported operational desired_state '{}': expected running",
                service.desired_state
            );
        }
    }
    Ok(())
}

fn trusted_container_mapping<'a>(
    cfg: &'a Config,
    service_id: &str,
) -> Option<&'a DispatchContainerMapping> {
    cfg.dispatch.as_ref()?.container_mappings.iter().find(|mapping| {
        mapping
            .service_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(service_id))
            && !mapping.container_name.trim().is_empty()
            && mapping
                .artifact
                .as_deref()
                .is_some_and(|artifact| !artifact.trim().is_empty())
    })
}

fn inspect_container(container_name: &str) -> Result<ContainerState> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_name])
        .output()
        .with_context(|| format!("execute docker inspect for trusted container '{container_name}'"))?;
    if output.status.success() {
        return match String::from_utf8_lossy(&output.stdout).trim() {
            "true" => Ok(ContainerState::Running),
            "false" => Ok(ContainerState::Stopped),
            other => bail!("unexpected docker inspect state '{other}' for '{container_name}'"),
        };
    }
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if combined.contains("No such object") || combined.contains("No such container") {
        Ok(ContainerState::Missing)
    } else {
        bail!("Docker unavailable while inspecting '{}': {}", container_name, combined.trim())
    }
}

fn execute_trusted_container_intent(
    cfg: &Config,
    service_id: &str,
    observed_before: ContainerState,
) -> Result<(String, String, bool)> {
    let mapping = trusted_container_mapping(cfg, service_id).ok_or_else(|| {
        anyhow::anyhow!(
            "no trusted local container mapping with image exists for service '{}'",
            service_id
        )
    })?;
    let container_name = mapping.container_name.trim();
    let image = mapping
        .artifact
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("trusted mapping requires non-empty artifact image");

    match observed_before {
        ContainerState::Running => Ok((
            "already_running".to_string(),
            format!("trusted container '{}' is already running", container_name),
            false,
        )),
        ContainerState::Stopped => {
            let output = Command::new("docker")
                .args(["start", container_name])
                .output()
                .with_context(|| format!("execute docker start for '{container_name}'"))?;
            if !output.status.success() {
                bail!(
                    "docker start failed for '{}': {}",
                    container_name,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Ok((
                "started".to_string(),
                format!(
                    "started trusted existing container '{}'; image mapping='{}'",
                    container_name, image
                ),
                true,
            ))
        }
        ContainerState::Missing => {
            let output = Command::new("docker")
                .args(["run", "-d", "--name", container_name, image])
                .output()
                .with_context(|| format!("execute governed docker run for '{container_name}'"))?;
            if !output.status.success() {
                bail!(
                    "governed docker run failed for '{}' using trusted image '{}': {}",
                    container_name,
                    image,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Ok((
                "deployed".to_string(),
                format!(
                    "created and started trusted container '{}' from local mapping image '{}'",
                    container_name, image
                ),
                true,
            ))
        }
    }
}

fn publish_role_snapshot(
    cfg: &Config,
    node_id: &str,
    hostname_override: Option<&str>,
    dserver_base: &str,
    project_id: &str,
    timeout_s: u64,
) -> Result<()> {
    let mut observation_cfg = cfg.clone();
    if let Some(auto_recovery) = observation_cfg.auto_recovery.as_mut() {
        auto_recovery.enabled = false;
        auto_recovery.dry_run = Some(true);
    }
    if let Some(desired_state) = observation_cfg.desired_state.as_mut() {
        desired_state.enabled = false;
    }

    let snapshot = run_cycle_for_node_identity(
        node_id,
        hostname_override,
        &observation_cfg.agent.schema_version,
        &observation_cfg,
    )?;
    let output = HttpJsonOutput::new(
        format!("{}/api/v1/snapshots", dserver_base.trim_end_matches('/')),
        project_id.trim().to_string(),
        timeout_s,
    )?;
    output.emit(&snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_requests_action_when_desired_service_is_missing() {
        let result = run_embedded_reconciler(true, false).unwrap();
        assert_eq!(result.status, "executed");
        assert_eq!(result.action_code, 1);
        assert!(!result.host_imports_allowed);
        assert!(!result.guest_side_effects_performed);
    }

    #[test]
    fn wasm_requests_no_action_when_service_is_already_running() {
        let result = run_embedded_reconciler(true, true).unwrap();
        assert_eq!(result.action_code, 0);
    }
}
