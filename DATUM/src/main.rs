use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::time::Instant;

mod action_results;
mod assisted_execution;
mod capabilities;
mod cli;
mod config;
mod core;
mod deployment_planning;
mod dispatch;
mod installation_plan;
mod installation_tasks;
mod local_mapping;
mod outputs;
mod wasm_runtime;

use cli::capabilities::run_capabilities_command;
use cli::deployment_planning::run_deployment_planning_command;
use cli::dispatch::run_dispatch_command;
use cli::installation_plan::run_installation_plan_command;
use cli::installation_tasks::run_installation_tasks_command;
use cli::show::show_command;
use cli::top::top_command;
use cli::wasm_runtime::run_wasm_test_command;
use core::cycle::run_cycle;
use outputs::file_json::FileJsonOutput;
use outputs::http_json::HttpJsonOutput;

fn main() -> Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();

    // Se o primeiro argumento "real" for um subcomando conhecido, usa a CLI nova.
    // Senão, mantém o comportamento antigo (run por padrão).
    if raw_args.len() >= 2 {
        match raw_args[1].as_str() {
            "top"
            | "show"
            | "once"
            | "run"
            | "dispatch"
            | "capabilities"
            | "deployment-planning"
            | "deploy-plan"
            | "installation-plan"
            | "install-plan"
            | "installation-tasks"
            | "install-tasks"
            | "wasm-test" => {
                let cli = Cli::parse();
                return dispatch_cli(cli);
            }
            _ => {}
        }
    }

    // Compatibilidade com o modo antigo:
    let args = LegacyRunArgs::parse();
    run_command(
        &args.config,
        args.out,
        args.cycle_s,
        args.node_id,
        args.hostname,
    )
}

fn dispatch_cli(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Run(args) => run_command(
            &args.config,
            args.out,
            args.cycle_s,
            args.node_id,
            args.hostname,
        ),
        Command::Once(args) => once_command(&args.config, args.out, args.node_id, args.hostname),
        Command::Dispatch(args) => {
            dispatch_command(&args.config, args.execute, args.publish_results)
        }
        Command::Capabilities(args) => capabilities_command(&args.config, args.publish),
        Command::DeploymentPlanning(args) => deployment_planning_command(
            &args.config,
            args.node_id.as_deref(),
            args.project_id.as_deref(),
            args.url.as_deref(),
            args.json,
        ),
        Command::InstallationPlan(args) => installation_plan_command(
            &args.config,
            args.node_id.as_deref(),
            args.project_id.as_deref(),
            args.url.as_deref(),
            args.json,
        ),
        Command::InstallationTasks(args) => installation_tasks_command(
            &args.config,
            args.node_id.as_deref(),
            args.project_id.as_deref(),
            args.url.as_deref(),
            args.task_id.as_deref(),
            args.request_id.as_deref(),
            args.claim_id.as_deref(),
            args.local_mapping_preflight,
            args.publish_local_mapping_preflight,
            args.local_mapping_recommendations,
            args.local_mapping_patch_proposals,
            args.write_local_mapping_patch.as_deref(),
            args.overwrite_local_mapping_patch,
            args.revalidate_local_mapping_patch.as_deref(),
            args.execution_state,
            args.readiness,
            args.execution_requests,
            args.execution_request,
            args.request_assisted_execution,
            args.execution_claims,
            args.execution_claim,
            args.claim_assisted_execution,
            args.claim_status.as_deref(),
            args.execution_attempts,
            args.execution_attempt,
            args.attempt_assisted_execution,
            args.execution_mode.as_deref(),
            args.approvals,
            args.approval,
            args.approve,
            args.approved_by.as_deref(),
            args.requested_by.as_deref(),
            args.claimed_by.as_deref(),
            args.attempted_by.as_deref(),
            args.message.as_deref(),
            args.status.as_deref(),
            args.reason.as_deref(),
            args.expires_at_utc.as_deref(),
            args.lease_expires_at_utc.as_deref(),
            args.materialize,
            args.json,
        ),
        Command::WasmTest(args) => wasm_test_command(&args.add, args.json),
        Command::Top(args) => top_command(&args.config, args.refresh_ms),
        Command::Show(args) => show_command(&args.config, args.target),
    }
}

#[derive(Parser, Debug)]
#[command(name = "datum")]
#[command(about = "DATUM core agent")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Run(RunArgs),
    Once(OnceArgs),
    Dispatch(DispatchArgs),
    Capabilities(CapabilitiesArgs),
    #[command(name = "deployment-planning", alias = "deploy-plan")]
    DeploymentPlanning(DeploymentPlanningArgs),
    #[command(name = "installation-plan", alias = "install-plan")]
    InstallationPlan(InstallationPlanArgs),
    #[command(name = "installation-tasks", alias = "install-tasks")]
    InstallationTasks(InstallationTasksArgs),
    #[command(name = "wasm-test")]
    WasmTest(WasmTestArgs),
    Top(TopArgs),
    Show(ShowArgs),
}

#[derive(Args, Debug)]
struct WasmTestArgs {
    #[arg(long, num_args = 2, value_names = ["A", "B"])]
    add: Vec<i32>,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Args, Debug)]
struct RunArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    cycle_s: Option<u64>,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,
}

#[derive(Args, Debug)]
struct OnceArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,
}

#[derive(Args, Debug)]
struct TopArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    refresh_ms: Option<u64>,
}

#[derive(Args, Debug)]
struct DispatchArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long, default_value_t = false)]
    execute: bool,

    #[arg(long, default_value_t = false)]
    publish_results: bool,
}

#[derive(Args, Debug)]
struct CapabilitiesArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long, default_value_t = false)]
    publish: bool,
}

#[derive(Args, Debug)]
struct DeploymentPlanningArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    project_id: Option<String>,

    #[arg(long)]
    url: Option<String>,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Args, Debug)]
struct InstallationPlanArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    project_id: Option<String>,

    #[arg(long)]
    url: Option<String>,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Args, Debug)]
struct InstallationTasksArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    project_id: Option<String>,

    #[arg(long)]
    url: Option<String>,

    #[arg(long)]
    task_id: Option<String>,

    #[arg(long)]
    request_id: Option<String>,

    #[arg(long)]
    claim_id: Option<String>,

    #[arg(long, default_value_t = false)]
    local_mapping_preflight: bool,

    #[arg(long, default_value_t = false)]
    publish_local_mapping_preflight: bool,

    #[arg(long, default_value_t = false)]
    local_mapping_recommendations: bool,

    #[arg(long, default_value_t = false)]
    local_mapping_patch_proposals: bool,

    #[arg(long)]
    write_local_mapping_patch: Option<String>,

    #[arg(long, default_value_t = false)]
    overwrite_local_mapping_patch: bool,

    #[arg(long)]
    revalidate_local_mapping_patch: Option<String>,

    #[arg(long, default_value_t = false)]
    execution_state: bool,

    #[arg(long, default_value_t = false)]
    readiness: bool,

    #[arg(long, default_value_t = false)]
    execution_requests: bool,

    #[arg(long, default_value_t = false)]
    execution_request: bool,

    #[arg(long, default_value_t = false)]
    request_assisted_execution: bool,

    #[arg(long, default_value_t = false)]
    execution_claims: bool,

    #[arg(long, default_value_t = false)]
    execution_claim: bool,

    #[arg(long, default_value_t = false)]
    claim_assisted_execution: bool,

    #[arg(long)]
    claim_status: Option<String>,

    #[arg(long, default_value_t = false)]
    execution_attempts: bool,

    #[arg(long, default_value_t = false)]
    execution_attempt: bool,

    #[arg(long, default_value_t = false)]
    attempt_assisted_execution: bool,

    #[arg(long)]
    execution_mode: Option<String>,

    #[arg(long, default_value_t = false)]
    approvals: bool,

    #[arg(long, default_value_t = false)]
    approval: bool,

    #[arg(long, default_value_t = false)]
    approve: bool,

    #[arg(long)]
    approved_by: Option<String>,

    #[arg(long)]
    requested_by: Option<String>,

    #[arg(long)]
    claimed_by: Option<String>,

    #[arg(long)]
    attempted_by: Option<String>,

    #[arg(long)]
    message: Option<String>,

    #[arg(long)]
    status: Option<String>,

    #[arg(long)]
    reason: Option<String>,

    #[arg(long)]
    expires_at_utc: Option<String>,

    #[arg(long)]
    lease_expires_at_utc: Option<String>,

    #[arg(long, default_value_t = false)]
    materialize: bool,

    #[arg(long, default_value_t = false)]
    json: bool,
}

#[derive(Args, Debug)]
struct ShowArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[command(subcommand)]
    target: cli::commands::ShowCommand,
}

#[derive(Parser, Debug)]
struct LegacyRunArgs {
    #[arg(long, default_value = "config.toml")]
    config: String,

    #[arg(long)]
    out: Option<String>,

    #[arg(long)]
    cycle_s: Option<u64>,

    #[arg(long)]
    node_id: Option<String>,

    #[arg(long)]
    hostname: Option<String>,
}

fn run_command(
    config_path: &str,
    out_override: Option<String>,
    cycle_s_override: Option<u64>,
    node_id_override: Option<String>,
    hostname_override: Option<String>,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;

    let node_id = node_id_override.unwrap_or(cfg.agent.node_id.clone());
    let cycle_s = cycle_s_override.unwrap_or(cfg.agent.cycle_s);
    let hostname = hostname_override.or(cfg.agent.hostname.clone());

    let file_cfg = cfg
        .outputs
        .file_json
        .clone()
        .unwrap_or(config::FileJsonConfig {
            enabled: true,
            path: "snapshot.json".to_string(),
        });

    let out_path = out_override.unwrap_or(file_cfg.path);

    let file_output = if file_cfg.enabled {
        Some(FileJsonOutput::new(out_path))
    } else {
        None
    };

    let http_output = match cfg.outputs.http.clone() {
        Some(http_cfg) if http_cfg.enabled => {
            let project_id = http_cfg
                .project_id
                .clone()
                .unwrap_or_else(|| "default-project".to_string());

            Some(HttpJsonOutput::new(
                http_cfg.url.clone(),
                project_id,
                http_cfg.timeout_s.unwrap_or(10),
            )?)
        }
        _ => None,
    };

    loop {
        let t0 = Instant::now();

        let snapshot_res = run_cycle(
            &node_id,
            hostname.as_deref(),
            &cfg.agent.schema_version,
            &cfg,
        );

        match snapshot_res {
            Ok(snapshot) => {
                let mut out_status = String::new();

                if let Some(out) = &file_output {
                    match out.emit(&snapshot) {
                        Ok(_) => out_status.push_str("file_json=ok"),
                        Err(e) => out_status.push_str(&format!("file_json=err({})", e)),
                    }
                } else {
                    out_status.push_str("file_json=disabled");
                }

                if let Some(out) = &http_output {
                    if !out_status.is_empty() {
                        out_status.push_str(", ");
                    }

                    match out.emit(&snapshot) {
                        Ok(_) => out_status.push_str("http_json=ok"),
                        Err(e) => out_status.push_str(&format!("http_json=err({})", e)),
                    }
                } else {
                    if !out_status.is_empty() {
                        out_status.push_str(", ");
                    }
                    out_status.push_str("http_json=disabled");
                }

                let dur_ms = t0.elapsed().as_millis();
                println!(
                    "mode=run cycle={} dur_ms={} entities={} edges={} events={} actions={} out={}",
                    snapshot.meta.cycle_id,
                    dur_ms,
                    snapshot.entities.len(),
                    snapshot.edges.len(),
                    snapshot.events.len(),
                    snapshot.actions.len(),
                    out_status
                );
            }
            Err(e) => {
                let dur_ms = t0.elapsed().as_millis();
                println!("mode=run cycle=err dur_ms={} err={}", dur_ms, e);
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(cycle_s));
    }
}

fn once_command(
    config_path: &str,
    out_override: Option<String>,
    node_id_override: Option<String>,
    hostname_override: Option<String>,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;

    let node_id = node_id_override.unwrap_or(cfg.agent.node_id.clone());
    let hostname = hostname_override.or(cfg.agent.hostname.clone());

    let file_cfg = cfg
        .outputs
        .file_json
        .clone()
        .unwrap_or(config::FileJsonConfig {
            enabled: true,
            path: "snapshot.json".to_string(),
        });

    let out_path = out_override.unwrap_or(file_cfg.path);

    let file_output = if file_cfg.enabled {
        Some(FileJsonOutput::new(out_path))
    } else {
        None
    };

    let http_output = match cfg.outputs.http.clone() {
        Some(http_cfg) if http_cfg.enabled => {
            let project_id = http_cfg
                .project_id
                .clone()
                .unwrap_or_else(|| "default-project".to_string());

            Some(HttpJsonOutput::new(
                http_cfg.url.clone(),
                project_id,
                http_cfg.timeout_s.unwrap_or(10),
            )?)
        }
        _ => None,
    };

    let t0 = Instant::now();
    let snapshot = run_cycle(
        &node_id,
        hostname.as_deref(),
        &cfg.agent.schema_version,
        &cfg,
    )?;

    let mut out_status = String::new();

    if let Some(out) = &file_output {
        match out.emit(&snapshot) {
            Ok(_) => out_status.push_str("file_json=ok"),
            Err(e) => out_status.push_str(&format!("file_json=err({})", e)),
        }
    } else {
        out_status.push_str("file_json=disabled");
    }

    if let Some(out) = &http_output {
        if !out_status.is_empty() {
            out_status.push_str(", ");
        }

        match out.emit(&snapshot) {
            Ok(_) => out_status.push_str("http_json=ok"),
            Err(e) => out_status.push_str(&format!("http_json=err({})", e)),
        }
    } else {
        if !out_status.is_empty() {
            out_status.push_str(", ");
        }
        out_status.push_str("http_json=disabled");
    }

    let dur_ms = t0.elapsed().as_millis();
    println!(
        "mode=once cycle={} dur_ms={} entities={} edges={} events={} actions={} out={}",
        snapshot.meta.cycle_id,
        dur_ms,
        snapshot.entities.len(),
        snapshot.edges.len(),
        snapshot.events.len(),
        snapshot.actions.len(),
        out_status
    );

    Ok(())
}

fn dispatch_command(config_path: &str, execute: bool, publish_results: bool) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    run_dispatch_command(&cfg, execute, publish_results)
}

fn capabilities_command(config_path: &str, publish: bool) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    run_capabilities_command(&cfg, publish)
}

fn deployment_planning_command(
    config_path: &str,
    node_id: Option<&str>,
    project_id: Option<&str>,
    url: Option<&str>,
    json: bool,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    run_deployment_planning_command(&cfg, node_id, project_id, url, json)
}

fn installation_plan_command(
    config_path: &str,
    node_id: Option<&str>,
    project_id: Option<&str>,
    url: Option<&str>,
    json: bool,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    run_installation_plan_command(&cfg, node_id, project_id, url, json)
}

fn installation_tasks_command(
    config_path: &str,
    node_id: Option<&str>,
    project_id: Option<&str>,
    url: Option<&str>,
    task_id: Option<&str>,
    request_id: Option<&str>,
    claim_id: Option<&str>,
    local_mapping_preflight: bool,
    publish_local_mapping_preflight: bool,
    local_mapping_recommendations: bool,
    local_mapping_patch_proposals: bool,
    write_local_mapping_patch: Option<&str>,
    overwrite_local_mapping_patch: bool,
    revalidate_local_mapping_patch: Option<&str>,
    execution_state: bool,
    readiness: bool,
    execution_requests: bool,
    execution_request: bool,
    request_assisted_execution: bool,
    execution_claims: bool,
    execution_claim: bool,
    claim_assisted_execution: bool,
    claim_status: Option<&str>,
    execution_attempts: bool,
    execution_attempt: bool,
    attempt_assisted_execution: bool,
    execution_mode: Option<&str>,
    approvals: bool,
    approval: bool,
    approve: bool,
    approved_by: Option<&str>,
    requested_by: Option<&str>,
    claimed_by: Option<&str>,
    attempted_by: Option<&str>,
    message: Option<&str>,
    status: Option<&str>,
    reason: Option<&str>,
    expires_at_utc: Option<&str>,
    lease_expires_at_utc: Option<&str>,
    materialize: bool,
    json: bool,
) -> Result<()> {
    let cfg = config::load_config(config_path)?;
    run_installation_tasks_command(
        &cfg,
        config_path,
        node_id,
        project_id,
        url,
        task_id,
        request_id,
        claim_id,
        local_mapping_preflight,
        publish_local_mapping_preflight,
        local_mapping_recommendations,
        local_mapping_patch_proposals,
        write_local_mapping_patch,
        overwrite_local_mapping_patch,
        revalidate_local_mapping_patch,
        execution_state,
        readiness,
        execution_requests,
        execution_request,
        request_assisted_execution,
        execution_claims,
        execution_claim,
        claim_assisted_execution,
        claim_status,
        execution_attempts,
        execution_attempt,
        attempt_assisted_execution,
        execution_mode,
        approvals,
        approval,
        approve,
        approved_by,
        requested_by,
        claimed_by,
        attempted_by,
        message,
        status,
        reason,
        expires_at_utc,
        lease_expires_at_utc,
        materialize,
        json,
    )
}

fn wasm_test_command(add_args: &[i32], json: bool) -> Result<()> {
    run_wasm_test_command(add_args, json)
}
