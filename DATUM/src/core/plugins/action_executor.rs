use anyhow::{Context, Result};
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use crate::config::{Config, ServiceRecoverySpec};
use crate::core::ids;
use crate::core::model::{ActionRecord, Entity, Event, Snapshot};
use crate::core::plugin::Plugin;

pub struct ActionExecutorPlugin;

impl ActionExecutorPlugin {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone)]
struct ExecResult {
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    duration_ms: u64,
    mode: String,
    message: String,
}

#[derive(Debug, Clone)]
struct CommandSpec {
    command: String,
    working_dir: Option<String>,
    shell: bool,
    background: bool,
}

fn trunc(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut out = s[..max_len].to_string();
        out.push_str("...");
        out
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn mk_action_phase_id(cycle_id: &str, correlation_id: &str, phase: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let a = ids::sanitize_id_component(correlation_id);
    let p = ids::sanitize_id_component(phase);
    format!("action:{}:{}:{}", a, p, c)
}

fn mk_event_id(cycle_id: &str, correlation_id: &str, suffix: &str) -> String {
    let c = ids::sanitize_id_component(cycle_id);
    let a = ids::sanitize_id_component(correlation_id);
    let s = ids::sanitize_id_component(suffix);
    format!("event:{}:{}:{}", a, s, c)
}

fn find_entity_by_id<'a>(snapshot: &'a Snapshot, id: &str) -> Option<&'a Entity> {
    snapshot.entities.iter().find(|e| e.id == id)
}

fn resolve_container_runtime_name(snapshot: &Snapshot, target_id: &str) -> Option<String> {
    let container = find_entity_by_id(snapshot, target_id)?;
    if container.kind != "container" {
        return None;
    }

    if let Some(name) = container.attrs.get("name").and_then(|v| v.as_str()) {
        if !name.trim().is_empty() {
            return Some(name.to_string());
        }
    }

    if let Some(id) = container.attrs.get("container_id").and_then(|v| v.as_str()) {
        if !id.trim().is_empty() {
            return Some(id.to_string());
        }
    }

    None
}

fn dry_run_result(message: String) -> ExecResult {
    ExecResult {
        success: true,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        duration_ms: 0,
        mode: "dry_run".to_string(),
        message,
    }
}

fn skipped_result(message: String) -> ExecResult {
    ExecResult {
        success: true,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        duration_ms: 0,
        mode: "skipped".to_string(),
        message,
    }
}

fn failed_result(message: String) -> ExecResult {
    ExecResult {
        success: false,
        exit_code: None,
        stdout: String::new(),
        stderr: message.clone(),
        duration_ms: 0,
        mode: "failed_precheck".to_string(),
        message,
    }
}

fn run_program(program: &str, args: &[&str], working_dir: Option<&str>) -> Result<ExecResult> {
    let started = Instant::now();

    let mut cmd = Command::new(program);
    cmd.args(args);

    if let Some(dir) = working_dir {
        if !dir.trim().is_empty() {
            cmd.current_dir(Path::new(dir));
        }
    }

    let output = cmd
        .output()
        .with_context(|| format!("failed to execute command: {} {:?}", program, args))?;

    let duration_ms = started.elapsed().as_millis() as u64;
    let exit_code = output.status.code();
    let success = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(ExecResult {
        success,
        exit_code,
        stdout,
        stderr,
        duration_ms,
        mode: "executed".to_string(),
        message: format!("command executed: {} {:?}", program, args),
    })
}

fn spawn_program(program: &str, args: &[&str], working_dir: Option<&str>) -> Result<ExecResult> {
    let started = Instant::now();

    let mut cmd = Command::new(program);
    cmd.args(args);

    if let Some(dir) = working_dir {
        if !dir.trim().is_empty() {
            cmd.current_dir(Path::new(dir));
        }
    }

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn command: {} {:?}", program, args))?;

    let duration_ms = started.elapsed().as_millis() as u64;

    Ok(ExecResult {
        success: true,
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        duration_ms,
        mode: "spawned".to_string(),
        message: format!("command spawned with pid {}", child.id()),
    })
}

fn run_shell_command(command: &str, working_dir: Option<&str>) -> Result<ExecResult> {
    #[cfg(target_os = "windows")]
    {
        run_program("cmd", &["/C", command], working_dir)
    }

    #[cfg(not(target_os = "windows"))]
    {
        run_program("sh", &["-c", command], working_dir)
    }
}

fn spawn_shell_command(command: &str, working_dir: Option<&str>) -> Result<ExecResult> {
    #[cfg(target_os = "windows")]
    {
        let started = Instant::now();

        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "start", "", "/B", command]);

        if let Some(dir) = working_dir {
            if !dir.trim().is_empty() {
                cmd.current_dir(Path::new(dir));
            }
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn shell command: {}", command))?;

        let duration_ms = started.elapsed().as_millis() as u64;

        Ok(ExecResult {
            success: true,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms,
            mode: "spawned".to_string(),
            message: format!("shell command spawned with pid {}", child.id()),
        })
    }

    #[cfg(not(target_os = "windows"))]
    {
        let started = Instant::now();

        let mut cmd = Command::new("sh");
        cmd.args(["-c", command]);

        if let Some(dir) = working_dir {
            if !dir.trim().is_empty() {
                cmd.current_dir(Path::new(dir));
            }
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn shell command: {}", command))?;

        let duration_ms = started.elapsed().as_millis() as u64;

        Ok(ExecResult {
            success: true,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            duration_ms,
            mode: "spawned".to_string(),
            message: format!("shell command spawned with pid {}", child.id()),
        })
    }
}

fn split_command_line(command: &str) -> Option<(String, Vec<String>)> {
    let parts: Vec<String> = command
        .split_whitespace()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.is_empty() {
        return None;
    }

    let program = parts[0].clone();
    let args = parts.into_iter().skip(1).collect();
    Some((program, args))
}

fn run_command_spec(spec: &CommandSpec, dry_run: bool) -> ExecResult {
    if spec.command.trim().is_empty() {
        return failed_result("restart_command has empty command".to_string());
    }

    if dry_run {
        return dry_run_result(format!(
            "would execute command: '{}' shell={} working_dir={:?} background={}",
            spec.command, spec.shell, spec.working_dir, spec.background
        ));
    }

    let exec = if spec.background {
        if spec.shell {
            spawn_shell_command(&spec.command, spec.working_dir.as_deref())
        } else if let Some((program, args_owned)) = split_command_line(&spec.command) {
            let args_refs: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();
            spawn_program(&program, &args_refs, spec.working_dir.as_deref())
        } else {
            return failed_result("restart_command could not parse program/args".to_string());
        }
    } else if spec.shell {
        run_shell_command(&spec.command, spec.working_dir.as_deref())
    } else if let Some((program, args_owned)) = split_command_line(&spec.command) {
        let args_refs: Vec<&str> = args_owned.iter().map(|s| s.as_str()).collect();
        run_program(&program, &args_refs, spec.working_dir.as_deref())
    } else {
        return failed_result("restart_command could not parse program/args".to_string());
    };

    match exec {
        Ok(r) => r,
        Err(e) => failed_result(format!("restart_command failed: {}", e)),
    }
}

fn resolve_restart_command_spec(cfg: &Config, service_name: &str) -> Option<CommandSpec> {
    let svc = cfg.find_registry_service(service_name)?;
    let recovery = svc.recovery.as_ref()?;

    if norm(&recovery.policy) != "restart_command" {
        return None;
    }

    let command = resolve_platform_command(recovery);
    if command.trim().is_empty() {
        return None;
    }

    Some(CommandSpec {
        command,
        working_dir: recovery.working_dir.clone(),
        shell: recovery.shell.unwrap_or(false),
        background: true,
    })
}

fn resolve_platform_command(recovery: &ServiceRecoverySpec) -> String {
    #[cfg(target_os = "windows")]
    {
        recovery
            .command_windows
            .clone()
            .or_else(|| recovery.command_linux.clone())
            .unwrap_or_default()
    }

    #[cfg(not(target_os = "windows"))]
    {
        recovery
            .command_linux
            .clone()
            .or_else(|| recovery.command_windows.clone())
            .unwrap_or_default()
    }
}

fn execute_action(snapshot: &Snapshot, cfg: &Config, action: &ActionRecord) -> ExecResult {
    if action.suppressed {
        return skipped_result("action suppressed by planner".to_string());
    }

    match norm(&action.kind).as_str() {
        "noop" => skipped_result(format!("noop for service '{}'", action.service_name)),
        "escalate" => skipped_result(format!("escalate for service '{}'", action.service_name)),

        "restart_container" => {
            if action.target_kind != "container" {
                return failed_result(format!(
                    "restart_container requires target_kind=container, got '{}'",
                    action.target_kind
                ));
            }

            let container_name = match resolve_container_runtime_name(snapshot, &action.target_id) {
                Some(x) => x,
                None => {
                    return failed_result(format!(
                        "could not resolve container runtime name for target_id '{}'",
                        action.target_id
                    ));
                }
            };

            if action.dry_run {
                return dry_run_result(format!("would execute: docker restart {}", container_name));
            }

            match run_program("docker", &["restart", &container_name], None) {
                Ok(r) => r,
                Err(e) => failed_result(format!("docker restart failed: {}", e)),
            }
        }

        "start_container" => {
            let container_name = if action.target_kind == "container" {
                match resolve_container_runtime_name(snapshot, &action.target_id) {
                    Some(x) => x,
                    None => {
                        return failed_result(format!(
                            "could not resolve container runtime name for target_id '{}'",
                            action.target_id
                        ));
                    }
                }
            } else if action.target_kind == "container_name" {
                action.target_id.clone()
            } else {
                return failed_result(format!(
                    "start_container requires target_kind=container or container_name, got '{}'",
                    action.target_kind
                ));
            };

            if action.dry_run {
                return dry_run_result(format!("would execute: docker start {}", container_name));
            }

            match run_program("docker", &["start", &container_name], None) {
                Ok(r) => r,
                Err(e) => failed_result(format!("docker start failed: {}", e)),
            }
        }

        "restart_command" => {
            if action.target_kind != "service" {
                return failed_result(format!(
                    "restart_command requires target_kind=service, got '{}'",
                    action.target_kind
                ));
            }

            let spec = match resolve_restart_command_spec(cfg, &action.service_name) {
                Some(s) => s,
                None => {
                    return failed_result(format!(
                        "could not resolve restart_command spec from service_registry for service '{}'",
                        action.service_name
                    ));
                }
            };

            run_command_spec(&spec, action.dry_run)
        }

        other => failed_result(format!("unsupported action_kind '{}'", other)),
    }
}

impl Plugin for ActionExecutorPlugin {
    fn name(&self) -> &'static str {
        "action_executor"
    }

    fn apply(&self, snapshot: &mut Snapshot, cfg: &crate::config::Config) -> Result<()> {
        let observed_at = snapshot.meta.observed_at_utc;
        let cycle_id = snapshot.meta.cycle_id.clone();

        let actions: Vec<ActionRecord> = snapshot
            .actions
            .iter()
            .filter(|a| a.phase == "planned")
            .cloned()
            .collect();

        if actions.is_empty() {
            return Ok(());
        }

        let mut new_actions: Vec<ActionRecord> = Vec::new();
        let mut new_events: Vec<Event> = Vec::new();

        for action in actions {
            let started_id = mk_action_phase_id(&cycle_id, &action.correlation_id, "started");
            new_actions.push(ActionRecord {
                id: started_id,
                correlation_id: action.correlation_id.clone(),
                origin: action.origin.clone(),
                kind: action.kind.clone(),
                phase: "started".to_string(),
                status: if action.suppressed { "skipped" } else { "running" }.to_string(),
                service_id: action.service_id.clone(),
                service_name: action.service_name.clone(),
                target_kind: action.target_kind.clone(),
                target_id: action.target_id.clone(),
                reason: action.reason.clone(),
                message: Some(format!(
                    "Recovery action started for service '{}': action={} target_kind={} target_id={} dry_run={}",
                    action.service_name, action.kind, action.target_kind, action.target_id, action.dry_run
                )),
                dry_run: action.dry_run,
                suppressed: action.suppressed,
                exit_code: None,
                duration_ms: None,
                stdout: None,
                stderr: None,
                observed_at_utc: observed_at,
            });

            new_events.push(Event {
                id: mk_event_id(&cycle_id, &action.correlation_id, "recovery_started"),
                kind: "recovery_started".to_string(),
                severity: "warning".to_string(),
                target: action.service_id.clone(),
                message: format!(
                    "Recovery action started for service '{}': action={} target_kind={} target_id={} dry_run={}",
                    action.service_name, action.kind, action.target_kind, action.target_id, action.dry_run
                ),
                correlation_id: Some(action.correlation_id.clone()),
                source_plugin: Some("action_executor".to_string()),
                status: Some(if action.suppressed { "skipped" } else { "executing" }.to_string()),
                details: Some(json!({"service_name": action.service_name, "action_kind": action.kind, "target_kind": action.target_kind, "target_id": action.target_id, "dry_run": action.dry_run, "suppressed": action.suppressed, "origin": action.origin})),
                observed_at_utc: observed_at,
            });

            let result = execute_action(snapshot, cfg, &action);

            let final_status = match result.mode.as_str() {
                "dry_run" => "dry_run",
                "skipped" => "skipped",
                _ if result.success => "succeeded",
                _ => "failed",
            }
            .to_string();

            let finished_id = mk_action_phase_id(&cycle_id, &action.correlation_id, "finished");
            new_actions.push(ActionRecord {
                id: finished_id,
                correlation_id: action.correlation_id.clone(),
                origin: action.origin.clone(),
                kind: action.kind.clone(),
                phase: "finished".to_string(),
                status: final_status,
                service_id: action.service_id.clone(),
                service_name: action.service_name.clone(),
                target_kind: action.target_kind.clone(),
                target_id: action.target_id.clone(),
                reason: action.reason.clone(),
                message: Some(result.message.clone()),
                dry_run: action.dry_run,
                suppressed: action.suppressed,
                exit_code: result.exit_code,
                duration_ms: Some(result.duration_ms),
                stdout: Some(trunc(&result.stdout, 400)),
                stderr: Some(trunc(&result.stderr, 400)),
                observed_at_utc: observed_at,
            });

            let (kind, severity) = match result.mode.as_str() {
                "dry_run" => ("recovery_dry_run", "info"),
                "skipped" => ("recovery_skipped", "info"),
                _ if result.success => ("recovery_execution_succeeded", "warning"),
                _ => ("recovery_execution_failed", "error"),
            };

            new_events.push(Event {
                id: mk_event_id(&cycle_id, &action.correlation_id, kind),
                kind: kind.to_string(),
                severity: severity.to_string(),
                target: action.service_id.clone(),
                message: format!(
                    "Recovery action result for service '{}': action={} mode={} success={} msg={}",
                    action.service_name, action.kind, result.mode, result.success, result.message
                ),
                correlation_id: Some(action.correlation_id.clone()),
                source_plugin: Some("action_executor".to_string()),
                status: Some(match result.mode.as_str() { "dry_run" => "dry_run", "skipped" => "skipped", _ if result.success => "resolved", _ => "failed" }.to_string()),
                details: Some(json!({"service_name": action.service_name, "action_kind": action.kind, "target_kind": action.target_kind, "target_id": action.target_id, "mode": result.mode, "success": result.success, "exit_code": result.exit_code, "duration_ms": result.duration_ms, "stdout": trunc(&result.stdout, 400), "stderr": trunc(&result.stderr, 400), "message": result.message, "origin": action.origin})),
                observed_at_utc: observed_at,
            });
        }

        snapshot.actions.extend(new_actions);
        snapshot.events.extend(new_events);

        Ok(())
    }
}